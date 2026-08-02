use super::{
    APP_VERSION, LICENSE_ID, LaunchGate, ManualStopwatchCommandClient, NOTICE_TEXT,
    NotesCommandClient, OverlayState, SOURCE_REPOSITORY_URL, ViewportUpdate, about_close_button,
    about_content_size, about_visible, authoritative_snapshot, catalog_outside_click,
    confirmed_mode_event, controls_visible, discord_avatars_allowed, discord_gate,
    dispatch_manual_stopwatch_action, dispatch_notes_action, handle_catalog_outcome,
    interactive_scrim, log_catalog_settings_outcome, paint_about_window, paint_control_notices,
    paint_overlay_version, paint_widget_catalog, schedule_wayland_input_region_commit,
    settings_failure_target, stopwatch_repaint_after, twitch_emotes_allowed, twitch_gate,
    viewport_builder, viewport_update_changed, widget_actions_allowed, x11_scale_changed,
    x11_should_request_focus,
};
use crate::{
    branding::{BrandAssets, install_fonts},
    discord::client::{DiscordConnectionState, DiscordGate, DiscordSnapshot},
    icons::AppIcon,
    notes::{NotesCommand, NotesDocument, NotesError, NotesUpdate},
    placement::screen_position,
    preferences::OverlayPreferences,
    runtime::SnapshotUpdate,
    session_clock::SessionClock,
    twitch::{
        client::TwitchGate,
        model::{TwitchConnectionState, TwitchSnapshot},
    },
    widgets::{
        ManualStopwatchAction, NotesWidgetState, format_session_elapsed, install_theme,
        session_draggable as stopwatch_draggable, session_visible as stopwatch_visible,
    },
};
use eframe::egui::{FontFamily, RawInput, Rect as EguiRect, Shape, WindowLevel, pos2, vec2};
use overcrow_config::WidgetPosition;
use overcrow_protocol::{CoreSnapshot, GameWindow, OverlayMode, Rect};
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct RecordingManualStopwatchClient {
    actions: RefCell<Vec<ManualStopwatchAction>>,
}

#[derive(Default)]
struct RecordingNotesClient {
    commands: RefCell<Vec<NotesCommand>>,
    fail: bool,
}

impl NotesCommandClient for RecordingNotesClient {
    fn send_notes(&self, command: NotesCommand) -> Result<(), NotesError> {
        if self.fail {
            return Err(NotesError::repository("forced command failure"));
        }
        self.commands.borrow_mut().push(command);
        Ok(())
    }
}

fn ready_notes_state() -> NotesWidgetState {
    let mut state = NotesWidgetState::default();
    state.apply_update(NotesUpdate {
        document: NotesDocument::default(),
        save_pending: false,
        error: None,
        durability_warning: false,
    });
    state
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
    assert_eq!(APP_VERSION, env!("CARGO_PKG_VERSION"));
}

#[test]
fn interactive_version_is_painted_as_discreet_overlay_chrome() {
    let context = eframe::egui::Context::default();
    context.enable_accesskit();
    install_fonts(&context);
    install_theme(&context);
    let output = context.run_ui(
        RawInput {
            screen_rect: Some(EguiRect::from_min_size(
                pos2(0.0, 0.0),
                vec2(1_280.0, 720.0),
            )),
            ..RawInput::default()
        },
        |ui| {
            paint_overlay_version(ui.ctx());
        },
    );

    let labels = accessible_labels(&output);
    assert!(
        labels.iter().any(|label| label == APP_VERSION),
        "{labels:?}"
    );
}

#[test]
fn about_window_has_no_native_title_bar_and_ends_with_the_version() {
    let context = eframe::egui::Context::default();
    context.enable_accesskit();
    install_fonts(&context);
    install_theme(&context);
    let mut brand = BrandAssets::default();
    let output = context.run_ui(
        RawInput {
            screen_rect: Some(EguiRect::from_min_size(
                pos2(0.0, 0.0),
                vec2(1_280.0, 720.0),
            )),
            ..RawInput::default()
        },
        |ui| {
            let _ = paint_about_window(ui.ctx(), vec2(1_280.0, 720.0), &mut brand);
        },
    );
    let labels = accessible_labels(&output);

    assert!(!labels.iter().any(|label| label == "Close window"));
    assert!(labels.iter().any(|label| label == "Close"), "{labels:?}");
    assert!(!labels.iter().any(|label| label == "×"), "{labels:?}");
    assert!(
        labels.iter().any(|label| label.contains(APP_VERSION)),
        "{labels:?}"
    );
}

#[test]
fn about_close_uses_the_shared_icon_painter() {
    let context = eframe::egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    let output = context.run_ui(RawInput::default(), |ui| {
        let _ = about_close_button(ui);
    });
    let close_glyph = AppIcon::Close.glyph();
    let family = output
        .shapes
        .iter()
        .find_map(|shape| icon_font_family(&shape.shape, close_glyph));

    assert_eq!(
        family,
        Some(FontFamily::Proportional),
        "close glyph must not use the UI text font"
    );
}

fn icon_font_family(shape: &Shape, glyph: &str) -> Option<FontFamily> {
    match shape {
        Shape::Text(text) if text.galley.job.text == glyph => text
            .galley
            .job
            .sections
            .first()
            .map(|section| section.format.font_id.family.clone()),
        Shape::Vec(shapes) => shapes
            .iter()
            .find_map(|shape| icon_font_family(shape, glyph)),
        _ => None,
    }
}

fn accessible_labels(output: &eframe::egui::FullOutput) -> Vec<String> {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accessibility tree")
        .nodes
        .iter()
        .flat_map(|(_, node)| {
            [
                node.label().map(str::to_owned),
                node.value().map(str::to_owned),
            ]
        })
        .flatten()
        .collect()
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
fn twitch_settings_message_is_visible_in_the_bottom_control_notices() {
    let context = eframe::egui::Context::default();
    let output = context.run_ui(eframe::egui::RawInput::default(), |ui| {
        paint_control_notices(
            ui,
            None,
            None,
            Some("Could not save Twitch widget settings."),
        );
    });
    let labels = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            eframe::egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(labels.contains(&"Could not save Twitch widget settings."));
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

#[test]
fn modal_surfaces_block_actions_from_widgets_behind_them() {
    assert!(widget_actions_allowed(false, false));
    assert!(!widget_actions_allowed(true, false));
    assert!(!widget_actions_allowed(false, true));
    assert!(!widget_actions_allowed(true, true));
}

#[test]
fn about_content_is_bounded_for_large_and_small_game_viewports() {
    assert_eq!(
        about_content_size(vec2(1_920.0, 1_080.0)),
        vec2(460.0, 520.0)
    );
    assert_eq!(about_content_size(vec2(320.0, 300.0)), vec2(224.0, 140.0));
}

#[test]
fn app_dispatches_notes_only_in_interactive_mode_and_updates_optimistically() {
    let client = RecordingNotesClient::default();
    let mut state = ready_notes_state();
    let command = NotesCommand::UpdateNote {
        id: "note-1".to_owned(),
        title: "General".to_owned(),
        body: "saved".to_owned(),
    };

    dispatch_notes_action(&client, &mut state, OverlayMode::Passive, command.clone());
    assert!(client.commands.borrow().is_empty());
    assert!(
        state
            .document()
            .active_note()
            .expect("valid test document has an active note")
            .body
            .is_empty()
    );

    dispatch_notes_action(
        &client,
        &mut state,
        OverlayMode::Interactive,
        command.clone(),
    );
    assert_eq!(*client.commands.borrow(), [command]);
    assert_eq!(
        state
            .document()
            .active_note()
            .expect("valid test document has an active note")
            .body,
        "saved"
    );
    assert!(state.save_pending());
}

#[test]
fn rejected_notes_command_restores_the_prior_visible_state() {
    let client = RecordingNotesClient {
        fail: true,
        ..RecordingNotesClient::default()
    };
    let mut state = ready_notes_state();
    state.set_note_draft("keep this draft");

    dispatch_notes_action(
        &client,
        &mut state,
        OverlayMode::Interactive,
        NotesCommand::UpdateNote {
            id: "note-1".to_owned(),
            title: "General".to_owned(),
            body: "keep this draft".to_owned(),
        },
    );

    assert!(
        state
            .document()
            .active_note()
            .expect("valid test document has an active note")
            .body
            .is_empty()
    );
    assert_eq!(state.note_draft(), "keep this draft");
    assert!(!state.save_pending());
    assert!(
        state
            .message()
            .is_some_and(|message| message.contains("command failure"))
    );
}

mod catalog {
    use std::{cell::Cell, io};

    use eframe::egui::{
        self, Event, RawInput, Rect,
        accesskit::{Action as AccessKitAction, ActionRequest, TreeId},
        pos2, vec2,
    };
    use overcrow_config::{CommittedSettingsSaveError, WidgetId, WidgetPosition, WidgetProfile};
    use overcrow_logging::{Component, LoggerRuntime};
    use overcrow_protocol::OverlayMode;

    use crate::branding::{BrandAssets, install_fonts};
    use crate::widgets::{
        BUILTIN_WIDGETS, CATALOG_ERROR_MAX_CHARS, CatalogAction, CatalogActionOutcome,
        CatalogCommit, CatalogFailureCategory, CatalogLayout, WidgetManager, apply_catalog_action,
        catalog_visible, install_theme, paint_catalog, paint_gated_options, persist_profile_change,
    };

    use super::{
        catalog_outside_click, handle_catalog_outcome, log_catalog_settings_outcome,
        paint_widget_catalog, settings_failure_target,
    };

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
    fn catalog_closes_only_for_an_outside_click_after_it_was_already_open() {
        let surface = Rect::from_min_size(pos2(100.0, 80.0), vec2(500.0, 360.0));

        assert!(catalog_outside_click(
            true,
            false,
            Some(pos2(40.0, 40.0)),
            surface
        ));
        assert!(!catalog_outside_click(
            true,
            false,
            Some(pos2(200.0, 200.0)),
            surface
        ));
        assert!(!catalog_outside_click(
            true,
            true,
            Some(pos2(40.0, 40.0)),
            surface
        ));
        assert!(!catalog_outside_click(false, false, None, surface));
    }

    #[test]
    fn unavailable_provider_options_are_disabled_and_explained() {
        let context = egui::Context::default();
        let callback_enabled = Cell::new(true);
        let output = context.run_ui(RawInput::default(), |ui| {
            paint_gated_options(ui, false, "Available while Warframe is active.", |ui| {
                callback_enabled.set(ui.is_enabled());
                let _ = ui.button("Provider action");
            });
        });
        let text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(!callback_enabled.get());
        assert!(
            text.contains(&"Available while Warframe is active."),
            "{text:?}"
        );
    }

    #[test]
    fn catalog_layout_adapts_without_exceeding_the_game_viewport() {
        let wide = CatalogLayout::for_viewport(vec2(1_920.0, 1_080.0));
        assert_eq!(wide.columns, 2);
        assert_eq!(wide.width, 840.0);
        assert_eq!(wide.max_height, 640.0);

        let narrow = CatalogLayout::for_viewport(vec2(640.0, 480.0));
        assert_eq!(narrow.columns, 1);
        assert_eq!(narrow.width, 568.0);
        assert_eq!(narrow.max_height, 310.0);

        let tiny = CatalogLayout::for_viewport(vec2(300.0, 220.0));
        assert_eq!(tiny.columns, 1);
        assert_eq!(tiny.width, 228.0);
        assert!(tiny.max_height <= 220.0);
    }

    #[test]
    fn complete_catalog_surface_stays_inside_the_game_viewport() {
        for viewport in [
            vec2(300.0, 220.0),
            vec2(640.0, 480.0),
            vec2(1_236.0, 526.0),
            vec2(1_920.0, 1_080.0),
        ] {
            let screen = Rect::from_min_size(pos2(0.0, 0.0), viewport);
            let context = egui::Context::default();
            install_fonts(&context);
            install_theme(&context);
            let profile = WidgetProfile::default();
            let mut brand = BrandAssets::default();
            let surface = Cell::new(Rect::NOTHING);

            let _ = context.run_ui(
                RawInput {
                    screen_rect: Some(screen),
                    ..RawInput::default()
                },
                |ui| {
                    let (_, rect) = paint_widget_catalog(
                        ui.ctx(),
                        ui.max_rect().size(),
                        &mut brand,
                        &profile,
                        true,
                        None,
                    );
                    surface.set(rect);
                },
            );

            let surface = surface.get();
            assert!(screen.contains(surface.min), "{viewport:?}: {surface:?}");
            assert!(screen.contains(surface.max), "{viewport:?}: {surface:?}");
        }
    }

    #[test]
    fn catalog_cards_keep_a_compact_click_target() {
        let viewport = vec2(1_236.0, 526.0);
        let context = egui::Context::default();
        context.enable_accesskit();
        install_fonts(&context);
        install_theme(&context);
        let profile = WidgetProfile::default();
        let mut brand = BrandAssets::default();
        let output = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), viewport)),
                ..RawInput::default()
            },
            |ui| {
                paint_widget_catalog(
                    ui.ctx(),
                    ui.max_rect().size(),
                    &mut brand,
                    &profile,
                    true,
                    None,
                );
            },
        );

        let nodes = output
            .platform_output
            .accesskit_update
            .expect("catalog accessibility tree")
            .nodes;
        let expected = BUILTIN_WIDGETS
            .iter()
            .map(|descriptor| {
                let verb = if profile.settings(descriptor.id).enabled {
                    "Disable"
                } else {
                    "Enable"
                };
                format!("{verb} {}", descriptor.name)
            })
            .collect::<std::collections::BTreeSet<_>>();
        let controls = nodes
            .into_iter()
            .filter_map(|(_, node)| {
                let label = node.label()?;
                expected
                    .contains(label)
                    .then_some((label.to_owned(), node.bounds()?))
            })
            .collect::<Vec<_>>();
        assert_eq!(controls.len(), BUILTIN_WIDGETS.len());
        let compressed = controls
            .into_iter()
            .filter(|(_, bounds)| bounds.y1 - bounds.y0 > 64.0)
            .collect::<Vec<_>>();
        assert!(compressed.is_empty(), "{compressed:?}");
    }

    #[test]
    fn non_warframe_catalog_omits_the_warframe_category_and_cards() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let output = context.run_ui(RawInput::default(), |ui| {
            let _ = paint_catalog(ui, &WidgetProfile::default(), None, false);
        });
        let labels = output
            .platform_output
            .accesskit_update
            .expect("catalog accessibility tree")
            .nodes
            .into_iter()
            .filter_map(|(_, node)| node.label().map(str::to_owned))
            .collect::<Vec<_>>();

        assert!(labels.iter().any(|label| label == "Enable Clock"));
        for label in [
            "Warframe",
            "Enable Warframe status",
            "Enable Void fissures",
            "Enable Warframe market",
            "Enable Sortie & Archon",
            "Enable Invasions",
        ] {
            assert!(
                !labels.iter().any(|candidate| candidate == label),
                "unexpected Warframe catalog label: {label}"
            );
        }
    }

    #[test]
    fn clicking_a_catalog_card_toggles_only_that_widget() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let profile = WidgetProfile::default();
        let first = context.run_ui(RawInput::default(), |ui| {
            let _ = paint_catalog(ui, &profile, None, true);
        });
        let clock_card = first
            .platform_output
            .accesskit_update
            .expect("catalog accessibility tree")
            .nodes
            .into_iter()
            .find_map(|(id, node)| (node.label() == Some("Enable Clock")).then_some(id))
            .expect("Clock catalog card");
        let click = Event::AccessKitActionRequest(ActionRequest {
            action: AccessKitAction::Click,
            target_tree: TreeId::ROOT,
            target_node: clock_card,
            data: None,
        });
        let mut actions = Vec::new();

        let _ = context.run_ui(
            RawInput {
                events: vec![click],
                ..RawInput::default()
            },
            |ui| actions = paint_catalog(ui, &profile, None, true),
        );

        assert_eq!(actions, [CatalogAction::SetEnabled(WidgetId::Clock, true)]);
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
    fn notes_sections_are_independent_but_cannot_both_be_hidden() {
        let mut profile = WidgetProfile::default();

        let first = apply_catalog_action(
            &mut profile,
            CatalogAction::SetNotesNoteVisible(false),
            |_| Ok(()),
        );
        assert!(matches!(first, CatalogActionOutcome::Durable(_)));
        assert!(!profile.notes_display.show_note);
        assert!(profile.notes_display.show_checklist);

        let second = apply_catalog_action(
            &mut profile,
            CatalogAction::SetNotesChecklistVisible(false),
            |_| Ok(()),
        );
        assert!(matches!(
            second,
            CatalogActionOutcome::RolledBack {
                category: CatalogFailureCategory::Validation,
                ..
            }
        ));
        assert!(profile.notes_display.show_checklist);
    }

    #[test]
    fn common_display_options_commit_through_the_widget_profile_store() {
        let mut profile = WidgetProfile::default();

        let clock = apply_catalog_action(
            &mut profile,
            CatalogAction::SetClockDateVisible(false),
            |_| Ok(()),
        );
        assert!(matches!(clock, CatalogActionOutcome::Durable(_)));
        assert!(!profile.clock_display.show_date);

        let performance = apply_catalog_action(
            &mut profile,
            CatalogAction::SetPerformanceLayout(overcrow_config::PerformanceLayout::Vertical),
            |_| Ok(()),
        );
        assert!(matches!(performance, CatalogActionOutcome::Durable(_)));
        assert_eq!(
            profile.performance_display.layout,
            overcrow_config::PerformanceLayout::Vertical
        );

        let participant_limit = apply_catalog_action(
            &mut profile,
            CatalogAction::SetDiscordParticipantLimit(12),
            |_| Ok(()),
        );
        assert!(matches!(
            participant_limit,
            CatalogActionOutcome::Durable(_)
        ));
        assert_eq!(profile.discord_voice_display.participant_limit, 12);

        let alignment = apply_catalog_action(
            &mut profile,
            CatalogAction::SetDiscordAlignment(overcrow_config::DiscordVoiceAlignment::Right),
            |_| Ok(()),
        );
        assert!(matches!(alignment, CatalogActionOutcome::Durable(_)));
        assert_eq!(
            profile.discord_voice_display.alignment,
            overcrow_config::DiscordVoiceAlignment::Right
        );
    }

    #[test]
    fn widget_scale_is_transactional_and_bounded_by_profile_validation() {
        let mut profile = WidgetProfile::default();

        let committed = apply_catalog_action(
            &mut profile,
            CatalogAction::SetScale(WidgetId::Media, 1.25),
            |_| Ok(()),
        );
        assert!(matches!(committed, CatalogActionOutcome::Durable(_)));
        assert_eq!(profile.media.scale, 1.25);

        let before = profile.clone();
        let rejected = apply_catalog_action(
            &mut profile,
            CatalogAction::SetScale(WidgetId::Media, 2.0),
            |_| Ok(()),
        );
        assert!(matches!(
            rejected,
            CatalogActionOutcome::RolledBack {
                category: CatalogFailureCategory::Validation,
                ..
            }
        ));
        assert_eq!(profile, before);
    }

    #[test]
    fn catalog_cards_are_compact_toggles_without_embedded_options() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let output = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1_200.0, 2_400.0))),
                ..RawInput::default()
            },
            |ui| {
                let _ = paint_catalog(ui, &WidgetProfile::default(), None, true);
            },
        );
        let text = output
            .platform_output
            .accesskit_update
            .expect("catalog accessibility tree")
            .nodes
            .into_iter()
            .flat_map(|(_, node)| {
                [
                    node.label().map(str::to_owned),
                    node.value().map(str::to_owned),
                ]
            })
            .flatten()
            .collect::<Vec<_>>();

        assert!(!text.iter().any(|value| value == "Show note"));
        assert!(!text.iter().any(|value| value == "Show checklist"));
        assert!(!text.iter().any(|value| value == "Connect Twitch"));
        assert!(!text.iter().any(|value| value == "Passive lifetime"));
        for removed in ["OVERLAY LIBRARY", "0 ACTIVE", "ON", "OFF"] {
            assert!(!text.iter().any(|value| value == removed), "{text:?}");
        }
        for removed in ["Passive", "Transparent", "More options"] {
            assert!(!text.iter().any(|value| value == removed), "{text:?}");
        }
        let profile = WidgetProfile::default();
        for descriptor in BUILTIN_WIDGETS {
            let verb = if profile.settings(descriptor.id).enabled {
                "Disable"
            } else {
                "Enable"
            };
            let label = format!("{verb} {}", descriptor.name);
            assert_eq!(
                text.iter().filter(|value| **value == label).count(),
                1,
                "{text:?}"
            );
        }
        assert!(text.iter().any(|value| value == "General"));
        assert!(text.iter().any(|value| value == "Warframe"));
        for descriptor in crate::widgets::BUILTIN_WIDGETS {
            assert!(
                text.iter().any(|value| value == descriptor.name),
                "missing catalog card for {}",
                descriptor.name
            );
        }
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
    fn reset_size_restores_defaults_without_resetting_scale_or_position() {
        let mut profile = WidgetProfile::default();
        profile.media.width = 540.0;
        profile.media.height = 220.0;
        profile.media.scale = 1.25;
        profile.media.position = WidgetPosition { x: 0.2, y: 0.3 };

        let outcome = apply_catalog_action(
            &mut profile,
            CatalogAction::ResetSize(WidgetId::Media),
            |_| Ok(()),
        );

        assert!(matches!(outcome, CatalogActionOutcome::Durable(_)));
        assert_eq!(
            (profile.media.width, profile.media.height),
            WidgetId::Media.default_panel_size()
        );
        assert_eq!(profile.media.scale, 1.25);
        assert_eq!(profile.media.position, WidgetPosition { x: 0.2, y: 0.3 });
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
    fn failed_geometry_save_restores_the_last_durable_profile() {
        let previous = WidgetProfile::default();
        let mut candidate = previous.clone();
        candidate.media.position = WidgetPosition { x: 0.8, y: 0.7 };

        let outcome = persist_profile_change(&mut candidate, previous.clone(), |_| {
            Err(io::Error::other("disk full"))
        });

        assert_eq!(candidate, previous);
        assert!(matches!(
            outcome,
            CatalogActionOutcome::RolledBack {
                category: CatalogFailureCategory::Filesystem,
                ..
            }
        ));
    }

    #[test]
    fn committed_geometry_save_keeps_the_new_profile_with_a_warning() {
        let previous = WidgetProfile::default();
        let mut candidate = previous.clone();
        candidate.media.position = WidgetPosition { x: 0.8, y: 0.7 };

        let outcome = persist_profile_change(&mut candidate, previous, |_| {
            Err(io::Error::other(CommittedSettingsSaveError::new(
                io::Error::other("forced parent sync failure"),
            )))
        });

        assert_eq!(candidate.media.position, WidgetPosition { x: 0.8, y: 0.7 });
        assert!(matches!(
            outcome,
            CatalogActionOutcome::CommittedWithWarning { .. }
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
fn x11_requests_focus_only_for_an_interactive_transition() {
    assert!(!x11_should_request_focus(true, None));
    assert!(x11_should_request_focus(
        true,
        Some(OverlayMode::Interactive)
    ));
}

#[test]
fn wayland_leaves_focus_to_the_compositor_bridge() {
    assert!(!x11_should_request_focus(
        false,
        Some(OverlayMode::Interactive)
    ));
}

#[test]
fn snapshot_update_tracks_game_geometry_and_input_mode() {
    assert_eq!(
        ViewportUpdate::from_snapshot(&snapshot(OverlayMode::Passive), true, 1.0),
        ViewportUpdate {
            mouse_passthrough: true,
            position: Some([100.0, 200.0]),
            size: Some([1920.0, 1080.0]),
        }
    );
    assert!(
        !ViewportUpdate::from_snapshot(&snapshot(OverlayMode::Interactive), true, 1.0)
            .mouse_passthrough
    );
}

#[test]
fn missing_core_authority_forces_passthrough_without_stale_geometry() {
    assert_eq!(
        ViewportUpdate::from_snapshot(&snapshot(OverlayMode::Interactive), false, 1.0),
        ViewportUpdate {
            mouse_passthrough: true,
            position: None,
            size: None,
        }
    );
}

#[test]
fn missing_core_authority_hides_retained_runtime_state() {
    let effective = authoritative_snapshot(&snapshot(OverlayMode::Interactive), false);

    assert!(effective.active_game.is_none());
    assert_eq!(effective.overlay_mode, OverlayMode::Passive);
}

#[test]
fn wayland_snapshot_leaves_geometry_to_the_compositor_bridge() {
    let mut wayland = snapshot(OverlayMode::Interactive);
    wayland.active_game.as_mut().expect("active game").backend = "wayland".to_owned();

    assert_eq!(
        ViewportUpdate::from_snapshot(&wayland, true, 2.0),
        ViewportUpdate {
            mouse_passthrough: false,
            position: None,
            size: None,
        }
    );
}

#[test]
fn wayland_input_region_change_schedules_a_follow_up_surface_commit() {
    let context = eframe::egui::Context::default();
    for _ in 0..3 {
        let _ = context.run_ui(RawInput::default(), |_| {});
    }
    assert!(!context.has_requested_repaint());
    let repaint_requests = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&repaint_requests);
    context.set_request_repaint_callback(move |_request| {
        recorded.fetch_add(1, Ordering::Relaxed);
    });

    schedule_wayland_input_region_commit(&context, false);
    schedule_wayland_input_region_commit(&context, true);

    assert_eq!(repaint_requests.load(Ordering::Relaxed), 1);
}

#[test]
fn elapsed_time_updates_do_not_reconfigure_the_viewport() {
    let previous = snapshot(OverlayMode::Passive);
    let mut current = previous.clone();
    current.session_elapsed_ms = Some(1_000);

    assert!(!viewport_update_changed(
        &previous,
        true,
        1.0,
        &ViewportUpdate::from_snapshot(&current, true, 1.0)
    ));
}

#[test]
fn x11_physical_geometry_is_converted_to_egui_points() {
    assert_eq!(
        ViewportUpdate::from_snapshot(&snapshot(OverlayMode::Passive), true, 2.0),
        ViewportUpdate {
            mouse_passthrough: true,
            position: Some([50.0, 100.0]),
            size: Some([960.0, 540.0]),
        }
    );
}

#[test]
fn invalid_scale_falls_back_to_one_without_affecting_wayland() {
    for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(
            ViewportUpdate::from_snapshot(&snapshot(OverlayMode::Passive), true, invalid),
            ViewportUpdate::from_snapshot(&snapshot(OverlayMode::Passive), true, 1.0)
        );
    }
}

#[test]
fn x11_geometry_is_reapplied_only_after_a_real_scale_change() {
    let x11 = snapshot(OverlayMode::Passive);
    assert!(x11_scale_changed(None, 2.0, &x11, true));
    assert!(!x11_scale_changed(Some(2.0), 2.0, &x11, true));
    assert!(x11_scale_changed(Some(1.0), 2.0, &x11, true));
    assert!(!x11_scale_changed(Some(1.0), 2.0, &x11, false));

    let mut wayland = x11;
    wayland.active_game.as_mut().expect("active game").backend = "wayland".to_owned();
    assert!(!x11_scale_changed(Some(1.0), 2.0, &wayland, true));
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

    let update = state.apply_snapshot(
        SnapshotUpdate::confirmed(snapshot(OverlayMode::Interactive), false),
        true,
        1.0,
    );

    assert!(state.passive_pending());
    assert_eq!(state.snapshot().overlay_mode, OverlayMode::Interactive);
    assert!(!update.mouse_passthrough);
}

#[test]
fn unconfirmed_failure_snapshot_does_not_release_the_escape_latch() {
    let mut state = OverlayState::from_snapshot(snapshot(OverlayMode::Interactive));
    let expected = state.snapshot().clone();
    state.begin_passive_request();

    let update = state.apply_snapshot(
        SnapshotUpdate::unconfirmed(CoreSnapshot::default()),
        true,
        1.0,
    );

    assert!(state.passive_pending());
    assert_eq!(state.snapshot(), &expected);
    assert!(!update.mouse_passthrough);
}

#[test]
fn confirmed_passive_releases_the_escape_latch() {
    let mut state = OverlayState::from_snapshot(snapshot(OverlayMode::Interactive));
    state.begin_passive_request();

    state.apply_snapshot(
        SnapshotUpdate::confirmed(snapshot(OverlayMode::Passive), true),
        true,
        1.0,
    );

    assert!(!state.passive_pending());
    assert_eq!(state.snapshot().overlay_mode, OverlayMode::Passive);
}

#[test]
fn interactive_can_reactivate_after_a_confirmed_passive() {
    let mut state = OverlayState::from_snapshot(snapshot(OverlayMode::Interactive));
    state.begin_passive_request();
    state.apply_snapshot(
        SnapshotUpdate::confirmed(snapshot(OverlayMode::Passive), true),
        true,
        1.0,
    );

    let update = state.apply_snapshot(
        SnapshotUpdate::confirmed(snapshot(OverlayMode::Interactive), false),
        true,
        1.0,
    );

    assert_eq!(state.snapshot().overlay_mode, OverlayMode::Interactive);
    assert!(!update.mouse_passthrough);
}

#[test]
fn twitch_verification_launch_is_single_flight() {
    let gate = LaunchGate::default();

    assert!(gate.try_acquire());
    assert!(!gate.try_acquire());
    assert!(gate.active());
    gate.release();
    assert!(!gate.active());
    assert!(gate.try_acquire());
}

#[test]
fn twitch_gate_fails_closed_when_core_authority_is_lost() {
    let gate = twitch_gate(false, true, true, Some("warframe".to_owned()));

    assert!(!gate.lifecycle_enabled);
    assert!(gate.active_game_authorized);
}

#[test]
fn twitch_emotes_require_the_exact_open_authorized_chat_gate() {
    let gate = twitch_gate(true, true, true, Some("warframe".to_owned()));
    let joined = TwitchSnapshot {
        channel: Some("warframe".to_owned()),
        connection: TwitchConnectionState::Joined,
        ..TwitchSnapshot::default()
    };

    assert!(twitch_emotes_allowed(&gate, &joined));
    assert!(!twitch_emotes_allowed(
        &TwitchGate {
            lifecycle_enabled: false,
            ..gate.clone()
        },
        &joined
    ));
    assert!(!twitch_emotes_allowed(
        &gate,
        &TwitchSnapshot {
            channel: Some("other".to_owned()),
            ..joined.clone()
        }
    ));
    assert!(!twitch_emotes_allowed(
        &gate,
        &TwitchSnapshot {
            connection: TwitchConnectionState::Reconnecting,
            ..joined
        }
    ));
}

#[test]
fn discord_gate_and_avatars_fail_closed_without_exact_runtime_authority() {
    let gate = discord_gate(true, true, true);
    let ready = DiscordSnapshot {
        connection: DiscordConnectionState::Ready,
        channel: Some(crate::discord::model::VoiceChannel {
            id: "9".to_owned(),
            name: "Squad".to_owned(),
            participants: Vec::new(),
        }),
        ..DiscordSnapshot::default()
    };

    assert!(discord_avatars_allowed(&gate, &ready));
    assert!(!discord_avatars_allowed(
        &DiscordGate {
            lifecycle_enabled: false,
            ..gate.clone()
        },
        &ready
    ));
    assert!(!discord_avatars_allowed(
        &gate,
        &DiscordSnapshot {
            channel: None,
            ..ready.clone()
        }
    ));
    assert!(discord_avatars_allowed(
        &gate,
        &DiscordSnapshot {
            connection: DiscordConnectionState::Connecting,
            ..ready
        }
    ));
}
