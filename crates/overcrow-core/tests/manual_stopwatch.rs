mod manual_stopwatch_tests {
    use std::collections::{BTreeSet, HashMap};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use overcrow_config::{LifecycleSettings, WidgetProfile};
    use overcrow_core::{CoreRuntime, ManualStopwatch, ProcessInfo, WindowObservation};
    use overcrow_protocol::{CoreState, ManualStopwatchSnapshot, Rect};
    use tokio::sync::RwLock;

    fn at(origin: Instant, seconds: u64) -> Instant {
        origin + Duration::from_secs(seconds)
    }

    #[test]
    fn manual_stopwatch_pause_and_resume_accumulate_only_running_time() {
        let origin = Instant::now();
        let mut timer = ManualStopwatch::default();
        timer.toggle(at(origin, 10));
        timer.toggle(at(origin, 15));
        timer.toggle(at(origin, 30));

        assert_eq!(
            timer.snapshot_at(at(origin, 32)),
            ManualStopwatchSnapshot {
                elapsed_ms: 7_000,
                running: true,
            }
        );
    }

    #[tokio::test]
    async fn manual_stopwatch_actions_are_ignored_without_an_active_game() {
        let origin = Instant::now();
        let runtime = runtime(enabled_profile(), HashMap::new()).await;

        let toggled = runtime.toggle_manual_stopwatch_at(origin).await;
        let reset = runtime.reset_manual_stopwatch_at(at(origin, 1)).await;

        assert_eq!(toggled.manual_stopwatch, ManualStopwatchSnapshot::default());
        assert_eq!(reset.manual_stopwatch, ManualStopwatchSnapshot::default());
    }

    #[tokio::test]
    async fn manual_stopwatch_actions_are_ignored_while_the_widget_is_disabled() {
        let origin = Instant::now();
        let runtime = active_runtime(WidgetProfile::default(), origin).await;

        let snapshot = runtime.toggle_manual_stopwatch_at(origin).await;

        assert_eq!(
            snapshot.manual_stopwatch,
            ManualStopwatchSnapshot::default()
        );
    }

    #[tokio::test]
    async fn manual_stopwatch_reset_and_game_exit_return_to_zero() {
        let origin = Instant::now();
        let runtime = active_runtime(enabled_profile(), origin).await;
        runtime.toggle_manual_stopwatch_at(origin).await;

        let reset = runtime.reset_manual_stopwatch_at(at(origin, 3)).await;
        assert_eq!(reset.manual_stopwatch, ManualStopwatchSnapshot::default());

        runtime.toggle_manual_stopwatch_at(at(origin, 4)).await;
        runtime.clear_game().await;
        assert_eq!(
            runtime.snapshot_at(at(origin, 7)).await.manual_stopwatch,
            ManualStopwatchSnapshot::default()
        );
    }

    #[tokio::test]
    async fn manual_stopwatch_survives_overlay_mode_changes() {
        let origin = Instant::now();
        let runtime = active_runtime(enabled_profile(), origin).await;
        runtime.toggle_manual_stopwatch_at(origin).await;

        runtime.toggle_overlay().await;
        runtime.toggle_overlay().await;

        assert_eq!(
            runtime.snapshot_at(at(origin, 2)).await.manual_stopwatch,
            ManualStopwatchSnapshot {
                elapsed_ms: 2_000,
                running: true,
            }
        );
    }

    #[tokio::test]
    async fn manual_stopwatch_transitions_and_paused_reset_publish_snapshot_events() {
        let origin = Instant::now();
        let runtime = active_runtime(enabled_profile(), origin).await;
        let mut snapshots = runtime.snapshots();

        runtime.toggle_manual_stopwatch_at(origin).await;
        snapshots.changed().await.expect("running transition");
        assert!(
            snapshots
                .borrow_and_update()
                .snapshot
                .manual_stopwatch
                .running
        );

        runtime.toggle_manual_stopwatch_at(at(origin, 3)).await;
        snapshots.changed().await.expect("paused transition");
        assert_eq!(
            snapshots.borrow_and_update().snapshot.manual_stopwatch,
            ManualStopwatchSnapshot {
                elapsed_ms: 3_000,
                running: false,
            }
        );

        runtime.reset_manual_stopwatch_at(at(origin, 4)).await;
        snapshots.changed().await.expect("paused reset");
        assert_eq!(
            snapshots.borrow_and_update().snapshot.manual_stopwatch,
            ManualStopwatchSnapshot::default()
        );
    }

    #[tokio::test]
    async fn running_manual_stopwatch_elapsed_drift_does_not_publish_snapshot_events() {
        let origin = Instant::now();
        let runtime = active_runtime(enabled_profile(), origin).await;
        runtime.toggle_manual_stopwatch_at(origin).await;
        let before = runtime.versioned_snapshot();
        let mut snapshots = runtime.snapshots();

        let point_in_time = runtime.snapshot_at(at(origin, 2)).await;

        assert_eq!(point_in_time.manual_stopwatch.elapsed_ms, 2_000);
        assert_eq!(runtime.versioned_snapshot(), before);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), snapshots.changed())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn manual_stopwatch_resets_when_the_active_process_instance_changes() {
        let origin = Instant::now();
        let runtime = active_runtime(enabled_profile(), origin).await;
        runtime.toggle_manual_stopwatch_at(origin).await;
        let mut replacement = sample_process();
        replacement.start_ticks += 1;

        runtime
            .install_process_snapshot_at(HashMap::from([(42, replacement)]), at(origin, 2))
            .await;

        assert_eq!(
            runtime.snapshot_at(at(origin, 2)).await.manual_stopwatch,
            ManualStopwatchSnapshot::default()
        );
    }

    #[tokio::test]
    async fn manual_stopwatch_profile_reload_is_validated_and_watched_without_snapshot_noise() {
        let origin = Instant::now();
        let runtime = active_runtime(WidgetProfile::default(), origin).await;
        let mut profiles = runtime.widget_profile();
        let mut snapshots = runtime.snapshots();
        let enabled = enabled_profile();

        runtime
            .reload_widget_profile(enabled.clone())
            .await
            .expect("valid profile reloads");
        profiles
            .changed()
            .await
            .expect("profile update is published");
        assert_eq!(*profiles.borrow_and_update(), enabled);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), snapshots.changed())
                .await
                .is_err()
        );

        runtime
            .reload_widget_profile(enabled.clone())
            .await
            .expect("unchanged valid profile reloads");
        assert!(
            tokio::time::timeout(Duration::from_millis(10), profiles.changed())
                .await
                .is_err()
        );
        assert_eq!(*runtime.widget_profile().borrow(), enabled);

        let mut invalid = enabled;
        invalid.schema_version += 1;
        assert!(runtime.reload_widget_profile(invalid).await.is_err());
        assert!(
            runtime
                .toggle_manual_stopwatch_at(origin)
                .await
                .manual_stopwatch
                .running
        );
    }

    fn enabled_profile() -> WidgetProfile {
        let mut profile = WidgetProfile::default();
        profile.manual_stopwatch.enabled = true;
        profile
    }

    async fn active_runtime(profile: WidgetProfile, now: Instant) -> CoreRuntime {
        let runtime = runtime(profile, HashMap::from([(42, sample_process())])).await;
        runtime
            .apply_bridge_observation_at(sample_observation(), now)
            .await;
        runtime
    }

    async fn runtime(profile: WidgetProfile, processes: HashMap<u32, ProcessInfo>) -> CoreRuntime {
        CoreRuntime::with_settings_and_widget_profile(
            Arc::new(RwLock::new(CoreState::default())),
            processes,
            LifecycleSettings {
                enabled: true,
                selected_steam_app_ids: BTreeSet::from([620]),
                ..LifecycleSettings::default()
            },
            profile,
        )
        .await
    }

    fn sample_process() -> ProcessInfo {
        ProcessInfo {
            pid: 42,
            parent_pid: 1,
            start_ticks: 100,
            timing: None,
            resources: Default::default(),
            name: "portal2".to_owned(),
            environment: HashMap::from([("SteamAppId".to_owned(), "620".to_owned())]),
            command_line: Vec::new(),
            executable: Some(PathBuf::from("/games/portal2")),
        }
    }

    fn sample_observation() -> WindowObservation {
        WindowObservation {
            pid: Some(42),
            app_id: Some("portal2".to_owned()),
            title: "Portal 2".to_owned(),
            rect: Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale: 1.0,
            backend: "wayland".to_owned(),
        }
    }
}
