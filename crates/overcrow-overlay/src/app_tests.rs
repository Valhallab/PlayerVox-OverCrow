use super::{
    LICENSE_ID, ManualStopwatchCommandClient, NOTICE_TEXT, OverlayState, SOURCE_REPOSITORY_URL,
    ViewportUpdate, about_visible, confirmed_mode_event, controls_visible,
    dispatch_manual_stopwatch_action, handle_catalog_outcome, interactive_scrim,
    log_catalog_settings_outcome, settings_failure_target, stopwatch_repaint_after,
    viewport_builder, viewport_update_changed,
};
use crate::{
    placement::screen_position,
    preferences::OverlayPreferences,
    runtime::SnapshotUpdate,
    session_clock::SessionClock,
    widgets::{
        ManualStopwatchAction, format_session_elapsed, session_draggable as stopwatch_draggable,
        session_visible as stopwatch_visible,
    },
};
use eframe::egui::{Rect as EguiRect, WindowLevel, pos2, vec2};
use overcrow_config::WidgetPosition;
use overcrow_protocol::{CoreSnapshot, GameWindow, OverlayMode, Rect};
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

#[derive(Default)]
struct RecordingManualStopwatchClient {
    actions: RefCell<Vec<ManualStopwatchAction>>,
}

impl ManualStopwatchCommandClient for RecordingManualStopwatchClient {
    fn toggle_manual_stopwatch(&self) {
        self.actions
            .borrow_mut()
            .push(ManualStopwatchAction::Toggle);
    }

    fn reset_manual_stopwatch(&self) {
        self.actions.borrow_mut().push(ManualStopwatchAction::Reset);
    }
}

fn snapshot(mode: OverlayMode) -> CoreSnapshot {
    CoreSnapshot {
        active_game: Some(GameWindow {
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
        }),
        overlay_mode: mode,
        session_elapsed_ms: None,
        ..CoreSnapshot::default()
    }
}

#[test]
fn about_copy_exposes_license_origin_and_public_source() {
    assert_eq!(LICENSE_ID, "AGPL-3.0-only");
    assert!(NOTICE_TEXT.lines().any(|line| {
        line == "OverCrow was originally created by Valhallab SASU and distributed under the PlayerVox brand."
    }));
    assert_eq!(
        SOURCE_REPOSITORY_URL,
        "https://github.com/Valhallab/PlayerVox-OverCrow"
    );
}

#[test]
fn about_panel_is_available_only_in_an_active_interactive_overlay() {
    assert!(about_visible(&snapshot(OverlayMode::Interactive), true));
    assert!(!about_visible(&snapshot(OverlayMode::Interactive), false));
    assert!(!about_visible(&snapshot(OverlayMode::Passive), true));

    let mut inactive = snapshot(OverlayMode::Interactive);
    inactive.active_game = None;
    assert!(!about_visible(&inactive, true));
}

#[test]
fn passive_is_logged_only_after_core_confirmation() {
    let unconfirmed = SnapshotUpdate::unconfirmed(snapshot(OverlayMode::Passive));
    let confirmed = SnapshotUpdate::confirmed(snapshot(OverlayMode::Passive), true);

    assert_eq!(
        confirmed_mode_event(OverlayMode::Interactive, true, &unconfirmed),
        None
    );
    assert_eq!(
        confirmed_mode_event(
            OverlayMode::Passive,
            false,
            &SnapshotUpdate::unconfirmed(snapshot(OverlayMode::Interactive)),
        ),
        None
    );
    assert_eq!(
        confirmed_mode_event(OverlayMode::Interactive, true, &confirmed),
        Some(OverlayMode::Passive)
    );
    assert_eq!(
        confirmed_mode_event(
            OverlayMode::Passive,
            false,
            &SnapshotUpdate::confirmed(snapshot(OverlayMode::Interactive), false),
        ),
        Some(OverlayMode::Interactive)
    );
}

#[test]
fn app_dispatches_manual_stopwatch_actions_to_the_exact_client_methods() {
    let client = RecordingManualStopwatchClient::default();
    let mut clock = crate::widgets::ManualStopwatchClock::default();
    let now = Instant::now();

    dispatch_manual_stopwatch_action(
        &client,
        &mut clock,
        OverlayMode::Interactive,
        Some(ManualStopwatchAction::Toggle),
        now,
    );
    dispatch_manual_stopwatch_action(
        &client,
        &mut clock,
        OverlayMode::Interactive,
        Some(ManualStopwatchAction::Reset),
        now,
    );

    assert_eq!(
        *client.actions.borrow(),
        [ManualStopwatchAction::Toggle, ManualStopwatchAction::Reset]
    );
    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(now + Duration::from_secs(1)),
        Duration::ZERO
    );
}

mod catalog {
    use std::{cell::Cell, io};

    use overcrow_config::{CommittedSettingsSaveError, WidgetId, WidgetPosition, WidgetProfile};
    use overcrow_logging::{Component, LoggerRuntime};
    use overcrow_protocol::OverlayMode;

    use crate::widgets::{
        CATALOG_ERROR_MAX_CHARS, CatalogAction, CatalogActionOutcome, CatalogCommit,
        CatalogFailureCategory, WidgetManager, apply_catalog_action, catalog_visible,
    };

    use super::{handle_catalog_outcome, log_catalog_settings_outcome, settings_failure_target};

    #[test]
    fn settings_diagnostic_targets_and_categories_are_stable_and_private() {
        let action = CatalogAction::SetEnabled(WidgetId::Media, true);
        assert_eq!(action.widget_id(), WidgetId::Media);
        assert_eq!(
            settings_failure_target(Some(WidgetId::WarframeSortie)),
            "widget=warframe_sortie"
        );
        assert_eq!(settings_failure_target(None), "affected_widgets=layout");

        let temp = tempfile::tempdir().expect("create log directory");
        let log_runtime =
            LoggerRuntime::start_in(Component::Overlay, temp.path()).expect("start test logger");
        let logger = log_runtime.logger();
        log_catalog_settings_outcome(
            &logger,
            WidgetId::Media,
            &CatalogActionOutcome::CommittedWithWarning {
                commit: CatalogCommit {
                    reload_widget_settings: false,
                },
                message: "private durability detail".to_owned(),
            },
        );
        log_catalog_settings_outcome(
            &logger,
            WidgetId::WarframeSortie,
            &CatalogActionOutcome::RolledBack {
                message: "private filesystem detail".to_owned(),
                category: CatalogFailureCategory::Filesystem,
            },
        );
        drop(logger);
        drop(log_runtime);

        let contents =
            std::fs::read_to_string(temp.path().join("overlay.log")).expect("read diagnostic log");
        assert!(contents.contains("widget_settings_save_failed widget=media category=durability"));
        assert!(
            contents
                .contains("widget_settings_save_failed widget=warframe_sortie category=filesystem")
        );
        assert!(!contents.contains("private durability detail"));
        assert!(!contents.contains("private filesystem detail"));
    }

    #[test]
    fn catalog_is_visible_only_when_open_for_an_interactive_game() {
        assert!(catalog_visible(OverlayMode::Interactive, true, true));
        assert!(!catalog_visible(OverlayMode::Interactive, true, false));
        assert!(!catalog_visible(OverlayMode::Passive, true, true));
        assert!(!catalog_visible(OverlayMode::Interactive, false, true));
    }

    #[test]
    fn catalog_actions_validate_before_requesting_a_save() {
        let mut profile = WidgetProfile::default();
        profile.clock.position.x = f32::NAN;
        let save_called = Cell::new(false);

        let outcome = apply_catalog_action(
            &mut profile,
            CatalogAction::SetEnabled(WidgetId::Session, false),
            |_| {
                save_called.set(true);
                Ok(())
            },
        );

        assert!(!save_called.get());
        assert!(profile.session.enabled);
        assert!(profile.clock.position.x.is_nan());
        assert!(matches!(
            outcome,
            CatalogActionOutcome::RolledBack { message, .. }
                if message.contains("Invalid")
                    && message.chars().count() <= CATALOG_ERROR_MAX_CHARS
        ));
    }

    #[test]
    fn passive_visibility_changes_without_enabling_the_widget() {
        let mut profile = WidgetProfile::default();
        let mut saved = Vec::new();

        let outcome = apply_catalog_action(
            &mut profile,
            CatalogAction::SetPassive(WidgetId::Clock, true),
            |candidate| {
                saved.push(candidate.clone());
                Ok(())
            },
        );

        assert_eq!(saved, [profile.clone()]);
        assert!(!profile.clock.enabled);
        assert!(profile.clock.show_in_passive);
        assert_eq!(
            outcome,
            CatalogActionOutcome::Durable(CatalogCommit {
                reload_widget_settings: false,
            })
        );
    }

    #[test]
    fn transparent_background_changes_without_enabling_the_widget() {
        let mut profile = WidgetProfile::default();
        assert!(!profile.session.transparent_background);

        let outcome = apply_catalog_action(
            &mut profile,
            CatalogAction::SetTransparentBackground(WidgetId::Session, true),
            |_| Ok(()),
        );

        assert!(profile.session.enabled);
        assert!(profile.session.transparent_background);
        assert!(!profile.clock.transparent_background);
        assert_eq!(
            outcome,
            CatalogActionOutcome::Durable(CatalogCommit {
                reload_widget_settings: false,
            })
        );
    }

    #[test]
    fn reset_changes_only_the_selected_widget_position() {
        let mut profile = WidgetProfile::default();
        profile.session.position = WidgetPosition { x: 0.25, y: 0.75 };
        profile.media.position = WidgetPosition { x: 0.8, y: 0.2 };
        let session_position = profile.session.position;

        let outcome = apply_catalog_action(
            &mut profile,
            CatalogAction::ResetPosition(WidgetId::Media),
            |_| Ok(()),
        );

        assert_eq!(profile.session.position, session_position);
        assert_eq!(profile.media.position, WidgetPosition { x: 0.5, y: 0.0 });
        assert!(matches!(outcome, CatalogActionOutcome::Durable(_)));
    }

    #[test]
    fn failed_catalog_save_keeps_the_prior_profile_and_bounds_the_message() {
        let mut profile = WidgetProfile::default();
        let previous = profile.clone();
        let oversized_detail = "x".repeat(CATALOG_ERROR_MAX_CHARS * 2);

        let outcome = apply_catalog_action(
            &mut profile,
            CatalogAction::SetEnabled(WidgetId::Clock, true),
            |_| Err(io::Error::other(oversized_detail)),
        );

        assert_eq!(profile, previous);
        assert!(matches!(
            outcome,
            CatalogActionOutcome::RolledBack { message, .. }
                if message.chars().count() <= CATALOG_ERROR_MAX_CHARS
        ));
    }

    #[test]
    fn committed_durability_warning_publishes_candidate_and_requests_manual_reload() {
        let mut profile = WidgetProfile::default();
        let outcome = apply_catalog_action(
            &mut profile,
            CatalogAction::SetEnabled(WidgetId::ManualStopwatch, true),
            |_| {
                Err(io::Error::other(CommittedSettingsSaveError::new(
                    io::Error::other("forced parent sync failure"),
                )))
            },
        );

        assert!(profile.manual_stopwatch.enabled);
        assert!(matches!(
            outcome,
            CatalogActionOutcome::CommittedWithWarning {
                commit: CatalogCommit {
                    reload_widget_settings: true,
                },
                message,
            } if message.contains("durability")
                && message.chars().count() <= CATALOG_ERROR_MAX_CHARS
        ));
    }

    #[test]
    fn app_queues_client_reload_after_durable_or_committed_publish_but_not_rollback() {
        let cases = [
            (
                CatalogActionOutcome::Durable(CatalogCommit {
                    reload_widget_settings: true,
                }),
                true,
                false,
            ),
            (
                CatalogActionOutcome::CommittedWithWarning {
                    commit: CatalogCommit {
                        reload_widget_settings: true,
                    },
                    message: "durability uncertain".to_owned(),
                },
                true,
                true,
            ),
            (
                CatalogActionOutcome::RolledBack {
                    message: "failed before replace".to_owned(),
                    category: CatalogFailureCategory::Filesystem,
                },
                false,
                true,
            ),
        ];

        for (outcome, expects_reload, expects_message) in cases {
            let mut manager = WidgetManager::default();
            let reloads = Cell::new(0);

            handle_catalog_outcome(&mut manager, outcome, || reloads.set(reloads.get() + 1));

            assert_eq!(reloads.get() == 1, expects_reload);
            assert_eq!(manager.catalog_message().is_some(), expects_message);
        }
    }

    #[test]
    fn manual_stopwatch_enable_change_requests_reload_only_after_a_durable_save() {
        let mut profile = WidgetProfile::default();
        let outcome = apply_catalog_action(
            &mut profile,
            CatalogAction::SetEnabled(WidgetId::ManualStopwatch, true),
            |_| Ok(()),
        );

        assert_eq!(
            outcome,
            CatalogActionOutcome::Durable(CatalogCommit {
                reload_widget_settings: true,
            })
        );

        let previous = profile.clone();
        let failed = apply_catalog_action(
            &mut profile,
            CatalogAction::SetEnabled(WidgetId::ManualStopwatch, false),
            |_| Err(io::Error::other("disk full")),
        );

        assert!(matches!(failed, CatalogActionOutcome::RolledBack { .. }));
        assert_eq!(profile, previous);
    }
}

#[test]
fn viewport_starts_transparent_borderless_and_passive() {
    let viewport = viewport_builder(false);

    assert_eq!(
        viewport.app_id.as_deref(),
        Some("io.github.overcrow.Overlay")
    );
    assert_eq!(viewport.transparent, Some(true));
    assert_eq!(viewport.decorations, Some(false));
    assert_eq!(viewport.resizable, Some(true));
    assert_eq!(viewport.mouse_passthrough, Some(true));
    assert_eq!(viewport.window_level, None);
}

#[test]
fn x11_viewport_requests_the_portable_always_on_top_hint() {
    let viewport = viewport_builder(true);

    assert_eq!(viewport.window_level, Some(WindowLevel::AlwaysOnTop));
}

#[test]
fn snapshot_update_tracks_game_geometry_and_input_mode() {
    assert_eq!(
        ViewportUpdate::from_snapshot(&snapshot(OverlayMode::Passive)),
        ViewportUpdate {
            mouse_passthrough: true,
            position: Some([100.0, 200.0]),
            size: Some([1920.0, 1080.0]),
        }
    );
    assert!(!ViewportUpdate::from_snapshot(&snapshot(OverlayMode::Interactive)).mouse_passthrough);
}

#[test]
fn wayland_snapshot_leaves_geometry_to_the_compositor_bridge() {
    let mut wayland = snapshot(OverlayMode::Interactive);
    wayland.active_game.as_mut().expect("active game").backend = "wayland".to_owned();

    assert_eq!(
        ViewportUpdate::from_snapshot(&wayland),
        ViewportUpdate {
            mouse_passthrough: false,
            position: None,
            size: None,
        }
    );
}

#[test]
fn elapsed_time_updates_do_not_reconfigure_the_viewport() {
    let previous = snapshot(OverlayMode::Passive);
    let mut current = previous.clone();
    current.session_elapsed_ms = Some(1_000);

    assert!(!viewport_update_changed(
        &previous,
        &ViewportUpdate::from_snapshot(&current)
    ));
}

#[test]
fn scrim_is_black_at_seventy_percent_only_while_interactive() {
    assert_eq!(
        interactive_scrim(&snapshot(OverlayMode::Interactive)),
        Some(eframe::egui::Color32::from_black_alpha(178))
    );
    assert_eq!(interactive_scrim(&snapshot(OverlayMode::Passive)), None);

    let mut without_game = snapshot(OverlayMode::Interactive);
    without_game.active_game = None;
    assert_eq!(interactive_scrim(&without_game), None);
}

#[test]
fn stopwatch_is_hidden_by_default_only_while_passive() {
    let preferences = OverlayPreferences::default();

    assert!(!stopwatch_visible(
        &snapshot(OverlayMode::Passive),
        &preferences
    ));
    assert!(stopwatch_visible(
        &snapshot(OverlayMode::Interactive),
        &preferences
    ));
}

#[test]
fn enabled_preference_shows_the_passive_stopwatch() {
    let mut preferences = OverlayPreferences::default();
    preferences.session.show_in_passive = true;

    assert!(stopwatch_visible(
        &snapshot(OverlayMode::Passive),
        &preferences
    ));
}

#[test]
fn stopwatch_is_hidden_without_an_active_game() {
    let mut without_game = snapshot(OverlayMode::Interactive);
    without_game.active_game = None;
    let mut preferences = OverlayPreferences::default();
    preferences.session.show_in_passive = true;

    assert!(!stopwatch_visible(&without_game, &preferences));
}

#[test]
fn stopwatch_is_draggable_only_for_an_interactive_game() {
    assert!(stopwatch_draggable(&snapshot(OverlayMode::Interactive)));
    assert!(!stopwatch_draggable(&snapshot(OverlayMode::Passive)));

    let mut without_game = snapshot(OverlayMode::Interactive);
    without_game.active_game = None;
    assert!(!stopwatch_draggable(&without_game));
}

#[test]
fn only_drag_release_requests_a_preference_save() {
    assert!(!crate::widgets::placement_save_requested(true, false));
    assert!(crate::widgets::placement_save_requested(false, true));
    assert!(!crate::widgets::placement_save_requested(false, false));
}

#[test]
fn normalized_placement_stays_inside_resized_viewports() {
    let position = WidgetPosition { x: 0.85, y: 0.4 };
    let widget = vec2(180.0, 80.0);
    let margin = 24.0;

    for viewport in [
        EguiRect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0)),
        EguiRect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)),
    ] {
        let top_left = screen_position(viewport, widget, margin, position);

        assert!(top_left.x >= viewport.min.x + margin);
        assert!(top_left.y >= viewport.min.y + margin);
        assert!(top_left.x + widget.x <= viewport.max.x - margin);
        assert!(top_left.y + widget.y <= viewport.max.y - margin);
    }
}

#[test]
fn controls_are_visible_only_for_an_interactive_game() {
    assert!(controls_visible(&snapshot(OverlayMode::Interactive)));
    assert!(!controls_visible(&snapshot(OverlayMode::Passive)));

    let mut without_game = snapshot(OverlayMode::Interactive);
    without_game.active_game = None;
    assert!(!controls_visible(&without_game));
}

#[test]
fn hidden_stopwatch_keeps_display_time_advancing() {
    let current = snapshot(OverlayMode::Passive);
    let preferences = OverlayPreferences::default();
    let now = Instant::now();
    let mut clock = SessionClock::default();

    assert!(!stopwatch_visible(&current, &preferences));
    clock.sync(Some(20_000), now);

    assert_eq!(
        clock.elapsed_at(now + Duration::from_secs(12)),
        Some(Duration::from_secs(32))
    );
}

#[test]
fn hidden_stopwatch_does_not_schedule_periodic_repaints() {
    let now = Instant::now();
    let mut clock = SessionClock::default();
    clock.sync(Some(20_000), now);

    assert_eq!(
        stopwatch_repaint_after(
            &snapshot(OverlayMode::Passive),
            &OverlayPreferences::default(),
            &clock,
            now,
        ),
        None
    );
    assert_eq!(
        stopwatch_repaint_after(
            &snapshot(OverlayMode::Interactive),
            &OverlayPreferences::default(),
            &clock,
            now,
        ),
        Some(Duration::from_secs(1))
    );
}

#[test]
fn elapsed_time_is_formatted_without_wrapping_hours() {
    assert_eq!(
        format_session_elapsed(Some(Duration::from_secs(0))),
        "00:00:00"
    );
    assert_eq!(
        format_session_elapsed(Some(Duration::from_secs(90_061))),
        "25:01:01"
    );
    assert_eq!(format_session_elapsed(None), "--:--:--");
}

#[test]
fn stale_interactive_after_escape_keeps_the_safe_interactive_surface() {
    let mut state = OverlayState::from_snapshot(snapshot(OverlayMode::Interactive));
    state.begin_passive_request();

    let update = state.apply_snapshot(SnapshotUpdate::confirmed(
        snapshot(OverlayMode::Interactive),
        false,
    ));

    assert!(state.passive_pending());
    assert_eq!(state.snapshot().overlay_mode, OverlayMode::Interactive);
    assert!(!update.mouse_passthrough);
}

#[test]
fn unconfirmed_failure_snapshot_does_not_release_the_escape_latch() {
    let mut state = OverlayState::from_snapshot(snapshot(OverlayMode::Interactive));
    let expected = state.snapshot().clone();
    state.begin_passive_request();

    let update = state.apply_snapshot(SnapshotUpdate::unconfirmed(CoreSnapshot::default()));

    assert!(state.passive_pending());
    assert_eq!(state.snapshot(), &expected);
    assert!(!update.mouse_passthrough);
}

#[test]
fn confirmed_passive_releases_the_escape_latch() {
    let mut state = OverlayState::from_snapshot(snapshot(OverlayMode::Interactive));
    state.begin_passive_request();

    state.apply_snapshot(SnapshotUpdate::confirmed(
        snapshot(OverlayMode::Passive),
        true,
    ));

    assert!(!state.passive_pending());
    assert_eq!(state.snapshot().overlay_mode, OverlayMode::Passive);
}

#[test]
fn interactive_can_reactivate_after_a_confirmed_passive() {
    let mut state = OverlayState::from_snapshot(snapshot(OverlayMode::Interactive));
    state.begin_passive_request();
    state.apply_snapshot(SnapshotUpdate::confirmed(
        snapshot(OverlayMode::Passive),
        true,
    ));

    let update = state.apply_snapshot(SnapshotUpdate::confirmed(
        snapshot(OverlayMode::Interactive),
        false,
    ));

    assert_eq!(state.snapshot().overlay_mode, OverlayMode::Interactive);
    assert!(!update.mouse_passthrough);
}
