use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overcrow_config::{
    LifecycleSettings, ManualGame, SettingsStore, WidgetProfile, WidgetSettingsStore,
};
use overcrow_core::{
    BRIDGE_WATCHDOG_INTERVAL, CoreRuntime, CoreService, OVERLAY_APP_ID, PROCESS_REFRESH_INTERVAL,
    ProcessInfo, ProcessTiming, WINDOW_POLL_INTERVAL, WindowObservation, WindowSource,
    apply_window_observation, poll_window_once, run_bridge_watchdog, run_process_refresh,
    should_use_x11_source,
};
use overcrow_protocol::{
    CoreSnapshot, CoreState, GameWindow, OverlayMode, Rect, VersionedCoreSnapshot,
};
use tokio::sync::RwLock;

#[tokio::test]
async fn toggle_overlay_returns_the_transitioned_snapshot_as_json() {
    let runtime = active_runtime().await;
    let service = CoreService::with_runtime(runtime);

    let json = service
        .toggle_overlay()
        .await
        .expect("in-memory state should serialize");
    let snapshot: CoreSnapshot = serde_json::from_str(&json).expect("valid snapshot JSON");

    assert_eq!(snapshot.overlay_mode, OverlayMode::Interactive);
    assert_eq!(snapshot.active_game, Some(sample_game()));
}

#[tokio::test]
async fn snapshot_versioned_preserves_the_raw_snapshot_contract() {
    let runtime = active_runtime().await;
    let service = CoreService::with_runtime(runtime.clone());
    let expected = runtime.versioned_snapshot();

    let versioned_json = service
        .snapshot_versioned()
        .await
        .expect("stable versioned snapshot should serialize");
    let raw_json = service
        .snapshot()
        .await
        .expect("raw snapshot should still serialize");

    assert_eq!(
        serde_json::from_str::<VersionedCoreSnapshot>(&versioned_json)
            .expect("versioned snapshot JSON"),
        expected
    );
    assert_eq!(
        serde_json::from_str::<CoreSnapshot>(&raw_json).expect("raw snapshot JSON"),
        expected.snapshot
    );
}

#[tokio::test]
async fn selected_wayland_observation_becomes_active() {
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, sample_process())]),
    )
    .await;

    let snapshot = runtime
        .apply_bridge_observation(sample_observation("portal2"))
        .await;

    assert_eq!(snapshot.active_game, Some(sample_game_with_backend("x11")));
}

#[tokio::test]
async fn session_elapsed_uses_process_age_and_advances() {
    let now = Instant::now();
    let mut process = sample_process();
    process.timing = Some(ProcessTiming::new(Duration::from_secs(1_200), now));
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, process)]),
    )
    .await;

    runtime
        .apply_bridge_observation_at(sample_observation("portal2"), now)
        .await;
    let snapshot = runtime.snapshot_at(now + Duration::from_secs(5)).await;

    assert_eq!(snapshot.session_elapsed_ms, Some(1_205_000));
}

#[tokio::test]
async fn semantic_snapshot_changes_increment_one_revision() {
    let runtime = active_runtime().await;
    let before = runtime.versioned_snapshot();
    runtime.set_overlay_interactive(true).await;
    let after = runtime.versioned_snapshot();
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(after.snapshot.overlay_mode, OverlayMode::Interactive);
}

#[tokio::test]
async fn point_in_time_clock_drift_does_not_change_the_published_revision() {
    let runtime = active_runtime().await;
    let before = runtime.versioned_snapshot();
    let _ = runtime
        .snapshot_at(Instant::now() + Duration::from_secs(2))
        .await;
    assert_eq!(runtime.versioned_snapshot(), before);
}

#[tokio::test]
async fn session_elapsed_is_unavailable_without_process_timing() {
    let now = Instant::now();
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, sample_process())]),
    )
    .await;

    runtime
        .apply_bridge_observation_at(sample_observation("portal2"), now)
        .await;

    assert_eq!(runtime.snapshot_at(now).await.session_elapsed_ms, None);
}

#[tokio::test]
async fn clearing_the_game_clears_session_elapsed() {
    let now = Instant::now();
    let mut process = sample_process();
    process.timing = Some(ProcessTiming::new(Duration::from_secs(1_200), now));
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, process)]),
    )
    .await;
    runtime
        .apply_bridge_observation_at(sample_observation("portal2"), now)
        .await;

    runtime.clear_game().await;

    assert_eq!(runtime.snapshot_at(now).await.session_elapsed_ms, None);
}

#[tokio::test]
async fn unselected_wayland_observation_clears_state() {
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([1623730]),
        HashMap::from([(42, sample_process())]),
    )
    .await;

    let snapshot = runtime
        .apply_bridge_observation(sample_observation("portal2"))
        .await;

    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[tokio::test]
async fn selected_x11_observation_becomes_active() {
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, sample_process())]),
    )
    .await;

    let snapshot = runtime
        .apply_x11_observation(Some(sample_observation("portal2")))
        .await;

    assert_eq!(snapshot.active_game, Some(sample_game()));
}

#[tokio::test]
async fn unselected_x11_observation_is_rejected() {
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([730]),
        HashMap::from([(42, sample_process())]),
    )
    .await;

    let snapshot = runtime
        .apply_x11_observation(Some(sample_observation("portal2")))
        .await;

    assert_eq!(snapshot.active_game, None);
}

#[tokio::test]
async fn manual_game_requires_the_exact_executable_path() {
    let settings = enabled_settings_with_manual("/games/portal2");
    let runtime =
        runtime_with_settings(settings.clone(), HashMap::from([(42, sample_process())])).await;
    assert!(
        runtime
            .apply_x11_observation(Some(sample_observation("portal2")))
            .await
            .active_game
            .is_some()
    );

    let mut helper = sample_process();
    helper.executable = Some(PathBuf::from("/games/portal2-helper"));
    let runtime = runtime_with_settings(settings, HashMap::from([(42, helper)])).await;

    assert_eq!(
        runtime
            .apply_x11_observation(Some(sample_observation("portal2")))
            .await
            .active_game,
        None
    );
}

#[tokio::test]
async fn disabled_and_invalid_settings_fail_closed() {
    let processes = HashMap::from([(42, sample_process())]);
    let disabled = runtime_with_settings(LifecycleSettings::default(), processes.clone()).await;
    assert_eq!(
        disabled
            .apply_x11_observation(Some(sample_observation("portal2")))
            .await
            .active_game,
        None
    );

    let mut invalid = enabled_settings_with_steam([620]);
    invalid.schema_version += 1;
    let invalid = runtime_with_settings(invalid, processes).await;
    assert_eq!(
        invalid
            .apply_x11_observation(Some(sample_observation("portal2")))
            .await
            .active_game,
        None
    );
}

#[tokio::test]
async fn construction_cannot_leave_unauthorized_state_when_the_state_lock_is_held() {
    let state = Arc::new(RwLock::new(CoreState::default()));
    let mut locked_state = state.write().await;
    locked_state.observe_game(sample_game());

    let construction = tokio::spawn(CoreRuntime::with_settings(
        Arc::clone(&state),
        HashMap::from([(42, sample_process())]),
        enabled_settings_with_steam([730]),
    ));
    tokio::task::yield_now().await;
    assert!(!construction.is_finished());
    drop(locked_state);
    let runtime = construction.await.expect("construction task");

    assert_eq!(runtime.snapshot().await.active_game, None);
}

#[tokio::test]
async fn reload_rejects_disabled_or_invalid_settings_without_replacing_authority() {
    let runtime = active_runtime().await;

    assert!(
        runtime
            .reload_settings(LifecycleSettings::default())
            .await
            .is_err()
    );
    let mut invalid = enabled_settings_with_steam([730]);
    invalid.schema_version += 1;
    assert!(runtime.reload_settings(invalid).await.is_err());

    assert_eq!(runtime.snapshot().await.active_game, Some(sample_game()));
}

#[tokio::test]
async fn reloading_a_valid_removed_selection_forces_passive() {
    let runtime = active_runtime().await;
    runtime.toggle_overlay().await;
    let no_selections = LifecycleSettings {
        enabled: true,
        ..LifecycleSettings::default()
    };

    let snapshot = runtime
        .reload_settings(no_selections)
        .await
        .expect("valid enabled settings may remove all selections");

    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[tokio::test]
async fn process_refresh_revalidates_selection_and_publishes_snapshot() {
    let runtime = active_runtime().await;
    runtime.toggle_overlay().await;
    let mut snapshots = runtime.snapshots();
    let mut replacement = sample_process();
    replacement
        .environment
        .insert("SteamAppId".to_owned(), "730".to_owned());

    runtime
        .install_process_snapshot(HashMap::from([(42, replacement)]))
        .await;

    snapshots.changed().await.expect("snapshot update");
    assert_eq!(snapshots.borrow().snapshot.active_game, None);
    assert_eq!(
        snapshots.borrow().snapshot.overlay_mode,
        OverlayMode::Passive
    );
}

#[tokio::test]
async fn process_refresh_rejects_a_reused_pid_even_when_the_new_identity_is_also_selected() {
    let mut settings = enabled_settings_with_manual("/games/portal2");
    settings.selected_steam_app_ids.insert(730);
    let mut native = sample_process();
    native.environment.clear();
    let runtime = runtime_with_settings(settings, HashMap::from([(42, native)])).await;
    runtime
        .apply_x11_observation(Some(sample_observation("portal2")))
        .await;
    assert!(runtime.snapshot().await.active_game.is_some());

    let mut replacement = sample_process();
    replacement
        .environment
        .insert("SteamAppId".to_owned(), "730".to_owned());
    replacement.executable = Some(PathBuf::from("/games/different-selected-game"));
    runtime
        .install_process_snapshot(HashMap::from([(42, replacement)]))
        .await;

    assert_eq!(runtime.snapshot().await.active_game, None);
}

#[tokio::test]
async fn process_refresh_rejects_same_identity_pid_reuse_by_start_time() {
    let runtime = active_runtime().await;
    runtime.toggle_overlay().await;
    let mut replacement = sample_process();
    replacement.start_ticks = 99;

    runtime
        .install_process_snapshot(HashMap::from([(42, replacement)]))
        .await;

    let snapshot = runtime.snapshot().await;
    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
    assert_eq!(snapshot.session_elapsed_ms, None);
}

#[tokio::test]
async fn lower_same_process_resample_cannot_rewind_session_elapsed() {
    let now = Instant::now();
    let mut process = sample_process();
    process.start_ticks = 10;
    process.timing = Some(ProcessTiming::new(Duration::from_secs(1_200), now));
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, process)]),
    )
    .await;
    runtime
        .apply_bridge_observation_at(sample_observation("portal2"), now)
        .await;

    let refresh_at = now + Duration::from_secs(5);
    let mut lower_sample = sample_process();
    lower_sample.start_ticks = 10;
    lower_sample.timing = Some(ProcessTiming::new(Duration::from_secs(1_190), refresh_at));
    runtime
        .install_process_snapshot_at(HashMap::from([(42, lower_sample)]), refresh_at)
        .await;

    assert_eq!(
        runtime.snapshot_at(refresh_at).await.session_elapsed_ms,
        Some(1_205_000)
    );
}

#[tokio::test]
async fn unavailable_resample_is_visible_without_losing_the_monotonic_floor() {
    let now = Instant::now();
    let mut process = sample_process();
    process.start_ticks = 10;
    process.timing = Some(ProcessTiming::new(Duration::from_secs(1_200), now));
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, process)]),
    )
    .await;
    runtime
        .apply_bridge_observation_at(sample_observation("portal2"), now)
        .await;

    let unavailable_at = now + Duration::from_secs(5);
    let mut unavailable = sample_process();
    unavailable.start_ticks = 10;
    runtime
        .install_process_snapshot_at(HashMap::from([(42, unavailable)]), unavailable_at)
        .await;
    assert_eq!(
        runtime.snapshot_at(unavailable_at).await.session_elapsed_ms,
        None
    );

    let recovered_at = now + Duration::from_secs(10);
    let mut lower_recovery = sample_process();
    lower_recovery.start_ticks = 10;
    lower_recovery.timing = Some(ProcessTiming::new(Duration::from_secs(1_195), recovered_at));
    runtime
        .install_process_snapshot_at(HashMap::from([(42, lower_recovery)]), recovered_at)
        .await;
    assert_eq!(
        runtime.snapshot_at(recovered_at).await.session_elapsed_ms,
        Some(1_210_000)
    );
}

#[tokio::test]
async fn reused_pid_new_process_instance_reports_its_own_age() {
    let now = Instant::now();
    let mut original = sample_process();
    original.start_ticks = 10;
    original.timing = Some(ProcessTiming::new(Duration::from_secs(1_200), now));
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, original)]),
    )
    .await;
    runtime
        .apply_bridge_observation_at(sample_observation("portal2"), now)
        .await;

    let replacement_at = now + Duration::from_secs(5);
    let mut replacement = sample_process();
    replacement.start_ticks = 20;
    replacement.timing = Some(ProcessTiming::new(Duration::from_secs(3), replacement_at));
    runtime
        .install_process_snapshot_at(HashMap::from([(42, replacement)]), replacement_at)
        .await;

    assert_eq!(runtime.snapshot_at(replacement_at).await.active_game, None);
    let new_session = runtime
        .apply_bridge_observation_at(sample_observation("portal2"), replacement_at)
        .await;
    assert_eq!(new_session.session_elapsed_ms, Some(3_000));
}

#[tokio::test]
async fn process_refresh_rejects_a_reused_pid_between_two_selected_manual_games() {
    let mut settings = enabled_settings_with_manual("/games/portal2");
    settings.manual_games.push(ManualGame {
        id: "another".to_owned(),
        name: "Another Game".to_owned(),
        executable: PathBuf::from("/games/another"),
    });
    let mut original = sample_process();
    original.environment.clear();
    let runtime = runtime_with_settings(settings, HashMap::from([(42, original)])).await;
    runtime
        .apply_x11_observation(Some(sample_observation("portal2")))
        .await;
    assert!(runtime.snapshot().await.active_game.is_some());

    let mut replacement = sample_process();
    replacement.environment.clear();
    replacement.executable = Some(PathBuf::from("/games/another"));
    runtime
        .install_process_snapshot(HashMap::from([(42, replacement)]))
        .await;

    assert_eq!(runtime.snapshot().await.active_game, None);
}

#[tokio::test]
async fn snapshot_watch_retains_latest_value_without_existing_receivers() {
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, sample_process())]),
    )
    .await;
    runtime
        .apply_x11_observation(Some(sample_observation("portal2")))
        .await;

    let snapshots = runtime.snapshots();

    assert_eq!(snapshots.borrow().snapshot.active_game, Some(sample_game()));
}

#[tokio::test]
async fn service_state_mutations_are_published_to_snapshot_watchers() {
    let runtime = active_runtime().await;
    let service = CoreService::with_runtime(runtime.clone());
    let mut snapshots = runtime.snapshots();

    service.toggle_overlay().await.expect("toggle");
    snapshots.changed().await.expect("toggle publication");
    assert_eq!(
        snapshots.borrow().snapshot.overlay_mode,
        OverlayMode::Interactive
    );

    service
        .set_overlay_interactive(false)
        .await
        .expect("set passive");
    snapshots.changed().await.expect("set publication");
    assert_eq!(
        snapshots.borrow().snapshot.overlay_mode,
        OverlayMode::Passive
    );

    service.clear_window().await.expect("clear");
    snapshots.changed().await.expect("clear publication");
    assert_eq!(snapshots.borrow().snapshot.active_game, None);
}

#[tokio::test]
async fn selected_process_watch_has_an_initial_value_and_only_changes_with_the_boolean() {
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, sample_process())]),
    )
    .await;
    let mut selected = runtime.selected_processes_running();
    assert!(*selected.borrow());

    runtime
        .install_process_snapshot(HashMap::from([(42, sample_process())]))
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), selected.changed())
            .await
            .is_err()
    );

    runtime.install_process_snapshot(HashMap::new()).await;
    selected.changed().await.expect("selected-running change");
    assert!(!*selected.borrow());
}

#[tokio::test]
async fn semantically_identical_process_refresh_does_not_publish_a_snapshot_change() {
    let runtime = active_runtime().await;
    let now = Instant::now();
    runtime
        .install_process_snapshot_at(HashMap::from([(42, sample_process())]), now)
        .await;
    runtime
        .install_process_snapshot_at(
            HashMap::from([(42, sample_process())]),
            now + Duration::from_secs(1),
        )
        .await;
    let mut snapshots = runtime.snapshots();

    runtime
        .install_process_snapshot_at(
            HashMap::from([(42, sample_process())]),
            now + Duration::from_secs(2),
        )
        .await;

    assert!(
        tokio::time::timeout(Duration::from_millis(10), snapshots.changed())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn semantically_identical_observations_do_not_publish_snapshot_changes() {
    let x11 = active_runtime().await;
    let mut x11_snapshots = x11.snapshots();
    x11.apply_x11_observation(Some(sample_observation("portal2")))
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), x11_snapshots.changed())
            .await
            .is_err()
    );

    let bridge = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, sample_process())]),
    )
    .await;
    bridge
        .apply_bridge_observation(sample_observation("portal2"))
        .await;
    let mut bridge_snapshots = bridge.snapshots();
    bridge
        .apply_bridge_observation(sample_observation("portal2"))
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), bridge_snapshots.changed())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn elapsed_progress_does_not_publish_a_state_only_snapshot_change() {
    let now = Instant::now();
    let mut process = sample_process();
    process.timing = Some(ProcessTiming::new(Duration::from_secs(1_200), now));
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, process)]),
    )
    .await;
    runtime
        .apply_bridge_observation_at(sample_observation("portal2"), now)
        .await;
    let mut snapshots = runtime.snapshots();

    let response = runtime
        .apply_bridge_observation_at(sample_observation("portal2"), now + Duration::from_secs(1))
        .await;

    assert_eq!(response.session_elapsed_ms, Some(1_201_000));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), snapshots.changed())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn idempotent_service_commands_do_not_publish_snapshot_changes() {
    let runtime = active_runtime().await;
    let service = CoreService::with_runtime(runtime.clone());
    let mut snapshots = runtime.snapshots();

    service
        .set_overlay_interactive(false)
        .await
        .expect("already passive");

    assert!(
        tokio::time::timeout(Duration::from_millis(10), snapshots.changed())
            .await
            .is_err()
    );

    let inert_runtime =
        CoreRuntime::new(Arc::new(RwLock::new(CoreState::default())), HashMap::new()).await;
    let inert_service = CoreService::with_runtime(inert_runtime.clone());
    let mut inert_snapshots = inert_runtime.snapshots();
    inert_service.toggle_overlay().await.expect("inert toggle");
    inert_service.clear_window().await.expect("already clear");
    assert!(
        tokio::time::timeout(Duration::from_millis(10), inert_snapshots.changed())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn reload_settings_service_loads_from_its_injected_store() {
    let runtime = active_runtime().await;
    runtime.toggle_overlay().await;
    let temp = tempfile::tempdir().expect("create test directory");
    let store = Arc::new(SettingsStore::from_path(temp.path().join("settings.json")));
    store
        .save(&enabled_settings_with_steam([730]))
        .expect("save replacement settings");
    let service = CoreService::with_runtime_and_store(runtime, store);

    let json = service.reload_settings().await.expect("reload succeeds");
    let snapshot: CoreSnapshot = serde_json::from_str(&json).expect("snapshot JSON");

    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[tokio::test]
async fn reload_settings_service_rejects_disabled_store_without_mutating_live_authority() {
    let runtime = active_runtime().await;
    let temp = tempfile::tempdir().expect("create test directory");
    let store = Arc::new(SettingsStore::from_path(temp.path().join("settings.json")));
    store
        .save(&LifecycleSettings::default())
        .expect("save disabled settings");
    let service = CoreService::with_runtime_and_store(runtime.clone(), store);

    assert!(service.reload_settings().await.is_err());
    assert_eq!(runtime.snapshot().await.active_game, Some(sample_game()));
}

#[cfg(unix)]
#[tokio::test]
async fn reload_settings_service_rejects_warning_load_without_mutating_live_authority() {
    use std::os::unix::fs::PermissionsExt;

    let runtime = active_runtime().await;
    let temp = tempfile::tempdir().expect("create test directory");
    let path = temp.path().join("settings.json");
    std::fs::write(&path, b"not json").expect("write malformed settings");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("make settings private");
    let service = CoreService::with_runtime_and_store(
        runtime.clone(),
        Arc::new(SettingsStore::from_path(path)),
    );

    assert!(service.reload_settings().await.is_err());
    assert_eq!(runtime.snapshot().await.active_game, Some(sample_game()));
}

#[tokio::test]
async fn reload_widget_settings_service_loads_only_from_its_injected_store() {
    let now = Instant::now();
    let runtime = active_runtime().await;
    let temp = tempfile::tempdir().expect("create test directory");
    let store = Arc::new(WidgetSettingsStore::from_paths(
        temp.path().join("widgets.json"),
        temp.path().join("overlay.json"),
    ));
    let mut profile = WidgetProfile::default();
    profile.manual_stopwatch.enabled = true;
    store.save(&profile).expect("save widget profile");
    let service = CoreService::with_runtime_and_widget_store(runtime.clone(), store);

    service
        .reload_widget_settings()
        .await
        .expect("trusted reload succeeds");
    let snapshot = runtime.toggle_manual_stopwatch_at(now).await;

    assert!(snapshot.manual_stopwatch.running);
}

#[tokio::test]
async fn reload_widget_settings_service_rejects_warning_without_replacing_live_authority() {
    let now = Instant::now();
    let runtime = active_runtime().await;
    let temp = tempfile::tempdir().expect("create test directory");
    let path = temp.path().join("widgets.json");
    let store = Arc::new(WidgetSettingsStore::from_paths(
        &path,
        temp.path().join("overlay.json"),
    ));
    let mut profile = WidgetProfile::default();
    profile.manual_stopwatch.enabled = true;
    store.save(&profile).expect("save widget profile");
    let service = CoreService::with_runtime_and_widget_store(runtime.clone(), store);
    service
        .reload_widget_settings()
        .await
        .expect("trusted reload succeeds");
    std::fs::write(path, b"not json").expect("replace profile with malformed contents");

    assert!(service.reload_widget_settings().await.is_err());
    assert!(
        runtime
            .toggle_manual_stopwatch_at(now)
            .await
            .manual_stopwatch
            .running
    );
}

#[tokio::test]
async fn report_window_creates_a_normalized_wayland_game() {
    let state = Arc::new(RwLock::new(CoreState::default()));
    let runtime = CoreRuntime::with_settings(
        state,
        HashMap::from([(sample_process().pid, sample_process())]),
        enabled_settings_with_steam([620]),
    )
    .await;
    let service = CoreService::with_runtime(runtime);

    let json = service
        .report_window(42, "Portal 2", "portal2", -100, 24, 1920, 1080, "1.25")
        .await
        .expect("in-memory state should serialize");
    let snapshot: CoreSnapshot = serde_json::from_str(&json).expect("valid snapshot JSON");

    assert_eq!(
        snapshot.active_game,
        Some(GameWindow {
            pid: Some(42),
            steam_app_id: Some(620),
            app_id: Some("portal2".to_owned()),
            title: "Portal 2".to_owned(),
            rect: Rect {
                x: -100,
                y: 24,
                width: 1920,
                height: 1080,
            },
            scale: 1.25,
            backend: "wayland".to_owned(),
        })
    );
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[tokio::test]
async fn x11_observation_is_classified_before_becoming_active() {
    let state = Arc::new(RwLock::new(CoreState::default()));
    let runtime = CoreRuntime::with_settings(
        Arc::clone(&state),
        HashMap::from([(42, sample_process())]),
        enabled_settings_with_steam([620]),
    )
    .await;

    apply_window_observation(&runtime, Some(sample_observation("portal2"))).await;

    let snapshot = state.read().await.snapshot().clone();
    assert_eq!(snapshot.active_game, Some(sample_game()));
}

#[tokio::test]
async fn focusing_the_overlay_does_not_replace_or_clear_the_game() {
    let state = Arc::new(RwLock::new(CoreState::default()));
    let runtime = CoreRuntime::new(Arc::clone(&state), HashMap::new()).await;
    state.write().await.observe_game(sample_game());
    state.write().await.toggle_overlay();

    apply_window_observation(&runtime, Some(sample_observation(OVERLAY_APP_ID))).await;

    let snapshot = state.read().await.snapshot().clone();
    assert_eq!(snapshot.active_game, Some(sample_game()));
    assert_eq!(snapshot.overlay_mode, OverlayMode::Interactive);
}

#[tokio::test]
async fn an_unclassified_x11_window_clears_the_game_and_forces_passive() {
    let state = Arc::new(RwLock::new(CoreState::default()));
    let runtime = CoreRuntime::new(Arc::clone(&state), HashMap::new()).await;
    state.write().await.observe_game(sample_game());
    state.write().await.toggle_overlay();

    apply_window_observation(&runtime, Some(sample_observation("org.example.Browser"))).await;

    let snapshot = state.read().await.snapshot().clone();
    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[tokio::test]
async fn a_missing_x11_observation_clears_the_game_and_forces_passive() {
    let state = Arc::new(RwLock::new(CoreState::default()));
    let runtime = CoreRuntime::new(Arc::clone(&state), HashMap::new()).await;
    state.write().await.observe_game(sample_game());
    state.write().await.toggle_overlay();

    apply_window_observation(&runtime, None).await;

    let snapshot = state.read().await.snapshot().clone();
    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[tokio::test]
async fn one_poll_reads_the_source_and_updates_state() {
    let state = Arc::new(RwLock::new(CoreState::default()));
    let runtime = CoreRuntime::with_settings(
        Arc::clone(&state),
        HashMap::from([(42, sample_process())]),
        enabled_settings_with_steam([620]),
    )
    .await;
    let mut source = OneShotSource(Some(sample_observation("portal2")));

    poll_window_once(&mut source, &runtime)
        .await
        .expect("the fake source succeeds");

    assert_eq!(
        state.read().await.snapshot().active_game,
        Some(sample_game())
    );
}

#[test]
fn polling_uses_the_required_cadence() {
    assert_eq!(WINDOW_POLL_INTERVAL, Duration::from_millis(250));
    assert_eq!(PROCESS_REFRESH_INTERVAL, Duration::from_secs(2));
    assert_eq!(BRIDGE_WATCHDOG_INTERVAL, Duration::from_millis(250));
}

#[test]
fn process_refresh_and_bridge_watchdog_have_independent_tasks() {
    let _ = run_process_refresh;
    let _ = run_bridge_watchdog;
}

#[test]
fn only_x11_sessions_use_a_local_window_source() {
    assert!(should_use_x11_source(Some("x11")));
    assert!(should_use_x11_source(Some("X11")));
    assert!(!should_use_x11_source(Some("wayland")));
    assert!(!should_use_x11_source(None));
}

fn sample_observation(app_id: &str) -> WindowObservation {
    WindowObservation {
        pid: Some(42),
        app_id: Some(app_id.to_owned()),
        title: "Portal 2".to_owned(),
        rect: Rect {
            x: 100,
            y: 200,
            width: 1920,
            height: 1080,
        },
        scale: 1.0,
        backend: "x11".to_owned(),
    }
}

fn sample_process() -> ProcessInfo {
    ProcessInfo {
        pid: 42,
        parent_pid: 1,
        start_ticks: 0,
        timing: None,
        resources: Default::default(),
        name: "portal2".to_owned(),
        environment: HashMap::from([("SteamAppId".to_owned(), "620".to_owned())]),
        command_line: vec!["portal2".to_owned()],
        executable: Some(PathBuf::from("/games/portal2")),
    }
}

struct OneShotSource(Option<WindowObservation>);

impl WindowSource for OneShotSource {
    fn active_window(&mut self) -> anyhow::Result<Option<WindowObservation>> {
        Ok(self.0.take())
    }
}

fn sample_game() -> GameWindow {
    GameWindow {
        pid: Some(42),
        steam_app_id: Some(620),
        app_id: Some("portal2".to_owned()),
        title: "Portal 2".to_owned(),
        rect: Rect {
            x: 100,
            y: 200,
            width: 1920,
            height: 1080,
        },
        scale: 1.0,
        backend: "x11".to_owned(),
    }
}

fn sample_game_with_backend(backend: &str) -> GameWindow {
    GameWindow {
        backend: backend.to_owned(),
        ..sample_game()
    }
}

fn enabled_settings_with_steam<const N: usize>(ids: [u32; N]) -> LifecycleSettings {
    LifecycleSettings {
        enabled: true,
        selected_steam_app_ids: BTreeSet::from(ids),
        ..LifecycleSettings::default()
    }
}

fn enabled_settings_with_manual(executable: &str) -> LifecycleSettings {
    LifecycleSettings {
        enabled: true,
        manual_games: vec![ManualGame {
            id: "portal2".to_owned(),
            name: "Portal 2".to_owned(),
            executable: PathBuf::from(executable),
        }],
        ..LifecycleSettings::default()
    }
}

async fn runtime_with_settings(
    settings: LifecycleSettings,
    processes: HashMap<u32, ProcessInfo>,
) -> CoreRuntime {
    CoreRuntime::with_settings(
        Arc::new(RwLock::new(CoreState::default())),
        processes,
        settings,
    )
    .await
}

async fn active_runtime() -> CoreRuntime {
    let runtime = runtime_with_settings(
        enabled_settings_with_steam([620]),
        HashMap::from([(42, sample_process())]),
    )
    .await;
    runtime
        .apply_x11_observation(Some(sample_observation("portal2")))
        .await;
    runtime
}
