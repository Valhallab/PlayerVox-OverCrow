use std::{collections::BTreeSet, sync::Arc, time::Instant};

use eframe::egui::{Event, PointerButton, RawInput, Rect, pos2, vec2};
use overcrow_config::{PerformanceLayout, TwitchPrefs, WidgetId, WidgetPosition, WidgetProfile};
use overcrow_protocol::{CoreSnapshot, GameWindow, OverlayMode, Rect as GameRect};

use super::{
    WidgetManager, layout::widget_index, placement_save_requested, widget_draggable, widget_visible,
};
use crate::{
    notes::NotesUpdate,
    twitch::model::{TwitchConnectionState, TwitchSnapshot},
    widgets::{
        BUILTIN_WIDGETS, NotesWidgetState, TwitchWidgetState, WidgetDescriptor,
        chrome::ResizeGripOutcome,
    },
};

const INTERACTIVE: OverlayMode = OverlayMode::Interactive;
const PASSIVE: OverlayMode = OverlayMode::Passive;

#[test]
fn registry_contains_every_stable_id_once() {
    let descriptors: &[WidgetDescriptor] = &BUILTIN_WIDGETS;
    let ids = descriptors
        .iter()
        .map(|item| item.id)
        .collect::<BTreeSet<_>>();

    assert_eq!(descriptors.len(), WidgetId::ALL.len());
    assert_eq!(ids, WidgetId::ALL.into_iter().collect());
    assert!(
        descriptors
            .iter()
            .all(|item| !item.name.is_empty() && !item.description.is_empty())
    );
}

#[test]
fn passive_mode_requires_both_enabled_and_passive_visibility() {
    let mut profile = WidgetProfile::default();
    profile.clock.enabled = true;

    assert!(!widget_visible(WidgetId::Clock, PASSIVE, true, &profile));

    profile.clock.show_in_passive = true;

    assert!(widget_visible(WidgetId::Clock, PASSIVE, true, &profile));
}

#[test]
fn visibility_requires_an_enabled_widget_and_active_game() {
    let mut profile = WidgetProfile::default();

    assert!(!widget_visible(
        WidgetId::Clock,
        INTERACTIVE,
        true,
        &profile
    ));

    profile.clock.enabled = true;

    assert!(widget_visible(WidgetId::Clock, INTERACTIVE, true, &profile));
    assert!(!widget_visible(
        WidgetId::Clock,
        INTERACTIVE,
        false,
        &profile
    ));
}

#[test]
fn all_widget_positions_stay_in_the_safe_area_after_resize() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let widget_size = vec2(180.0, 80.0);
    let margin = 24.0;

    for (index, id) in WidgetId::ALL.into_iter().enumerate() {
        let ratio = index as f32 / (WidgetId::ALL.len() - 1) as f32;
        profile.settings_mut(id).position = WidgetPosition {
            x: ratio,
            y: 1.0 - ratio,
        };
        manager.set_measured_size(id, INTERACTIVE, widget_size);
    }

    for viewport in [
        Rect::from_min_size(pos2(100.0, 200.0), vec2(1_920.0, 1_080.0)),
        Rect::from_min_size(pos2(40.0, 60.0), vec2(800.0, 600.0)),
    ] {
        for id in WidgetId::ALL {
            let top_left = manager.screen_position(id, INTERACTIVE, viewport, margin, &profile);

            assert!(top_left.x >= viewport.min.x + margin);
            assert!(top_left.y >= viewport.min.y + margin);
            assert!(top_left.x + widget_size.x <= viewport.max.x - margin);
            assert!(top_left.y + widget_size.y <= viewport.max.y - margin);
        }
    }
}

#[test]
fn untouched_enabled_primary_widget_positions_do_not_coincide() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    let widget_size = vec2(180.0, 80.0);
    let margin = 24.0;
    profile.clock.enabled = true;
    profile.performance.enabled = true;

    let ids = [WidgetId::Session, WidgetId::Clock, WidgetId::Performance];
    for id in ids {
        manager.set_measured_size(id, INTERACTIVE, widget_size);
    }
    let positions =
        ids.map(|id| manager.screen_position(id, INTERACTIVE, viewport, margin, &profile));

    assert_ne!(positions[0], positions[1]);
    assert_ne!(positions[0], positions[2]);
    assert_ne!(positions[1], positions[2]);
}

#[test]
fn manager_keeps_measured_sizes_and_catalog_state_transient() {
    let mut manager = WidgetManager::default();

    assert_eq!(
        manager.measured_size(WidgetId::Media, INTERACTIVE),
        eframe::egui::Vec2::ZERO
    );
    assert!(!manager.catalog_open());

    manager.set_measured_size(WidgetId::Media, INTERACTIVE, vec2(320.0, 140.0));
    manager.set_catalog_open(true);

    assert_eq!(
        manager.measured_size(WidgetId::Media, INTERACTIVE),
        vec2(320.0, 140.0)
    );
    assert!(manager.catalog_open());
}

#[test]
fn passive_measurements_do_not_replace_interactive_geometry() {
    let mut manager = WidgetManager::default();
    let id = WidgetId::Notes;

    manager.set_measured_size(id, INTERACTIVE, vec2(360.0, 280.0));
    manager.set_measured_size(id, PASSIVE, vec2(360.0, 112.0));

    assert_eq!(manager.measured_size(id, INTERACTIVE), vec2(360.0, 280.0));
    assert_eq!(manager.measured_size(id, PASSIVE), vec2(360.0, 112.0));
}

#[test]
fn mode_change_keeps_the_last_visible_top_left() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::Notes;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let margin = 24.0;
    profile.notes.position = WidgetPosition { x: 0.5, y: 1.0 };
    manager.set_measured_size(id, INTERACTIVE, vec2(360.0, 280.0));

    let interactive = manager.screen_position(id, INTERACTIVE, viewport, margin, &profile);
    manager.finish_drag_only(
        id,
        INTERACTIVE,
        viewport,
        margin,
        &mut profile,
        vec2(360.0, 280.0),
        interactive,
        false,
        false,
    );
    manager.set_measured_size(id, PASSIVE, vec2(360.0, 112.0));
    let passive = manager.screen_position(id, PASSIVE, viewport, margin, &profile);

    assert_eq!(passive, interactive);
}

#[test]
fn first_unmeasured_mode_change_keeps_the_last_visible_top_left() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::Media;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let margin = 24.0;
    let top_left = pos2(536.0, 120.0);

    manager.finish_drag_only(
        id,
        INTERACTIVE,
        viewport,
        margin,
        &mut profile,
        vec2(240.0, 80.0),
        top_left,
        false,
        false,
    );

    assert_eq!(
        manager.screen_position(id, PASSIVE, viewport, margin, &profile),
        top_left
    );
}

#[test]
fn content_size_change_keeps_the_last_visible_top_left() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::Media;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let margin = 24.0;
    let top_left = pos2(200.0, 120.0);

    manager.finish_drag_only(
        id,
        INTERACTIVE,
        viewport,
        margin,
        &mut profile,
        vec2(240.0, 80.0),
        top_left,
        false,
        false,
    );
    manager.set_measured_size(id, INTERACTIVE, vec2(520.0, 120.0));

    assert_eq!(
        manager.screen_position(id, INTERACTIVE, viewport, margin, &profile),
        top_left
    );
}

#[test]
fn game_viewport_resize_reprojects_the_runtime_anchor_from_saved_position() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::Media;
    let initial_viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let resized_viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_200.0, 800.0));
    let widget_size = vec2(240.0, 80.0);
    let margin = 24.0;
    profile.media.position = WidgetPosition { x: 0.75, y: 0.5 };

    let initial = manager.screen_position(id, INTERACTIVE, initial_viewport, margin, &profile);
    manager.finish_drag_only(
        id,
        INTERACTIVE,
        initial_viewport,
        margin,
        &mut profile,
        widget_size,
        initial,
        false,
        false,
    );

    let resized = manager.screen_position(id, INTERACTIVE, resized_viewport, margin, &profile);

    assert_eq!(
        resized,
        crate::placement::screen_position(
            resized_viewport,
            widget_size,
            margin,
            profile.media.position,
        )
    );
    assert_ne!(resized, initial);
}

#[test]
fn hovered_widget_exposes_the_shared_foreground_controls() {
    let context = eframe::egui::Context::default();
    context.enable_accesskit();
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let output = context.run_ui(
        eframe::egui::RawInput {
            screen_rect: Some(viewport),
            events: vec![eframe::egui::Event::PointerMoved(pos2(220.0, 180.0))],
            ..Default::default()
        },
        |ui| {
            manager.begin_widget_frame();
            manager.finish_drag_only(
                WidgetId::Session,
                INTERACTIVE,
                viewport,
                24.0,
                &mut profile,
                vec2(240.0, 80.0),
                pos2(200.0, 160.0),
                false,
                false,
            );
            let actions = manager.paint_widget_controls(ui.ctx(), viewport, &profile, |_, _| {});
            assert!(actions.is_empty());
        },
    );
    let labels = output
        .platform_output
        .accesskit_update
        .expect("toolbar accessibility tree")
        .nodes
        .into_iter()
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect::<Vec<_>>();

    assert!(labels.iter().any(|label| label == "Show in passive mode"));
    assert!(labels.iter().any(|label| label == "Widget options"));
    assert!(labels.iter().any(|label| label == "Disable widget"));
}

#[test]
fn measured_content_height_does_not_replace_stored_panel_height() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::WarframeMarket;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    profile.settings_mut(id).height = 400.0;

    let save_requested = manager.finish_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(320.0, 100.0),
        pos2(100.0, 100.0),
        false,
        false,
        ResizeGripOutcome::default(),
    );

    assert!(!save_requested);
    assert_eq!(profile.settings(id).height, 400.0);
    assert_eq!(manager.measured_size(id, INTERACTIVE), vec2(320.0, 100.0));
}

#[test]
fn content_height_widgets_keep_the_visible_top_left_after_resize_release() {
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    let margin = 24.0;
    let visible_top_left = pos2(620.0, 540.0);
    let rendered_size = vec2(360.0, 180.0);

    for id in [WidgetId::WarframeSortie, WidgetId::WarframeInvasions] {
        let mut manager = WidgetManager::default();
        let mut profile = WidgetProfile::default();
        profile.settings_mut(id).width = 320.0;
        profile.settings_mut(id).height = 400.0;
        manager.set_measured_size(id, INTERACTIVE, vec2(320.0, 160.0));

        manager.finish_resizable_panel(
            id,
            INTERACTIVE,
            viewport,
            margin,
            &mut profile,
            rendered_size,
            visible_top_left,
            false,
            false,
            ResizeGripOutcome {
                drag_delta: vec2(40.0, 80.0),
                dragging: true,
                drag_stopped: false,
                drag_cancelled: false,
            },
        );

        let release_save = manager.finish_resizable_panel(
            id,
            INTERACTIVE,
            viewport,
            margin,
            &mut profile,
            rendered_size,
            visible_top_left,
            false,
            false,
            ResizeGripOutcome {
                drag_delta: vec2(0.0, 0.0),
                dragging: false,
                drag_stopped: true,
                drag_cancelled: false,
            },
        );

        assert!(release_save);
        assert_eq!(profile.settings(id).width, 360.0);
        assert_eq!(profile.settings(id).height, 480.0);
        let mut fresh_manager = WidgetManager::default();
        let restored = fresh_manager.screen_position(id, INTERACTIVE, viewport, margin, &profile);
        assert!((restored.x - visible_top_left.x).abs() < 0.01);
        assert!((restored.y - visible_top_left.y).abs() < 0.01);

        let final_rendered_size = vec2(360.0, 220.0);
        let settle_save = manager.finish_resizable_panel(
            id,
            INTERACTIVE,
            viewport,
            margin,
            &mut profile,
            final_rendered_size,
            visible_top_left,
            false,
            false,
            ResizeGripOutcome::default(),
        );
        assert!(!settle_save);
        assert_eq!(manager.measured_size(id, INTERACTIVE), final_rendered_size);
        let next = manager.screen_position(id, INTERACTIVE, viewport, margin, &profile);
        assert!((next.x - visible_top_left.x).abs() < 0.01);
        assert!((next.y - visible_top_left.y).abs() < 0.01);
    }
}

#[test]
fn horizontal_performance_resize_changes_width_without_persisting_height() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::Performance;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    let top_left = pos2(200.0, 180.0);
    profile.performance.width = 580.0;
    profile.performance.height = 0.0;

    manager.finish_width_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(580.0, 120.0),
        top_left,
        false,
        false,
        ResizeGripOutcome {
            drag_delta: vec2(100.0, 80.0),
            dragging: true,
            drag_stopped: false,
            drag_cancelled: false,
        },
    );
    assert_eq!(profile.performance.width, 580.0);
    assert_eq!(manager.resize.expect("active resize session").size.x, 680.0);

    manager.finish_width_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(680.0, 120.0),
        top_left,
        false,
        false,
        ResizeGripOutcome {
            drag_delta: vec2(10.0, 0.0),
            dragging: true,
            drag_stopped: false,
            drag_cancelled: false,
        },
    );
    assert_eq!(profile.performance.width, 580.0);
    assert_eq!(manager.resize.expect("active resize session").size.x, 690.0);

    let release_save = manager.finish_width_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(690.0, 120.0),
        top_left,
        false,
        false,
        ResizeGripOutcome {
            drag_delta: vec2(0.0, 0.0),
            dragging: false,
            drag_stopped: true,
            drag_cancelled: false,
        },
    );

    assert!(release_save);
    assert_eq!(profile.performance.width, 690.0);
    assert_eq!(profile.performance.height, 0.0);
    assert_eq!(
        WidgetManager::default().screen_position(id, INTERACTIVE, viewport, 24.0, &profile),
        top_left
    );

    let settle_save = manager.finish_width_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(690.0, 96.0),
        top_left,
        false,
        false,
        ResizeGripOutcome::default(),
    );
    assert!(!settle_save);
    assert_eq!(
        manager.screen_position(id, INTERACTIVE, viewport, 24.0, &profile),
        top_left
    );
}

#[test]
fn vertical_performance_resize_can_shrink_below_the_horizontal_minimum() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::Performance;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    profile.performance_display.layout = PerformanceLayout::Vertical;
    profile.performance.width = 300.0;

    manager.finish_width_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(300.0, 240.0),
        pos2(200.0, 180.0),
        false,
        false,
        ResizeGripOutcome {
            drag_delta: vec2(-150.0, 0.0),
            dragging: true,
            drag_stopped: false,
            drag_cancelled: false,
        },
    );

    assert_eq!(manager.resize.expect("active resize session").size.x, 180.0);
}

#[test]
fn performance_resize_starts_at_the_active_layout_minimum() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::Performance;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    profile.performance_display.layout = PerformanceLayout::Horizontal;
    profile.performance.width = 180.0;

    manager.finish_width_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(300.0, 120.0),
        pos2(200.0, 180.0),
        false,
        false,
        ResizeGripOutcome {
            drag_delta: vec2(20.0, 0.0),
            dragging: true,
            drag_stopped: false,
            drag_cancelled: false,
        },
    );

    assert_eq!(manager.resize.expect("active resize session").size.x, 320.0);
}

#[test]
fn performance_renders_the_active_resize_width_before_release() {
    let context = eframe::egui::Context::default();
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::Performance;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    let top_left = pos2(200.0, 180.0);
    profile.performance.enabled = true;
    profile.performance.width = 580.0;
    profile.performance.position = WidgetPosition { x: 0.0, y: 0.0 };

    manager.finish_width_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(580.0, 120.0),
        top_left,
        false,
        false,
        ResizeGripOutcome {
            drag_delta: vec2(100.0, 0.0),
            dragging: true,
            drag_stopped: false,
            drag_cancelled: false,
        },
    );
    let snapshot = CoreSnapshot {
        active_game: Some(GameWindow {
            pid: Some(42),
            steam_app_id: Some(230_410),
            app_id: Some("game.exe".to_owned()),
            title: "Game".to_owned(),
            rect: GameRect {
                x: 0,
                y: 0,
                width: 1_920,
                height: 1_080,
            },
            scale: 1.0,
            backend: "test".to_owned(),
        }),
        overlay_mode: INTERACTIVE,
        ..CoreSnapshot::default()
    };

    let _ = context.run_ui(
        RawInput {
            screen_rect: Some(viewport),
            ..RawInput::default()
        },
        |ui| {
            manager.begin_widget_frame();
            manager.render_performance(ui, &snapshot, &mut profile, 24.0);
        },
    );

    let visible = manager.visible_rects[widget_index(id)].expect("performance widget rect");
    assert_eq!(profile.performance.width, 580.0);
    assert!((visible.width() - 680.0).abs() <= 0.5, "{visible:?}");
}

#[test]
fn only_a_stopped_drag_requests_persistence() {
    assert!(!placement_save_requested(true, false));
    assert!(placement_save_requested(false, true));
    assert!(!placement_save_requested(true, true));
    assert!(!placement_save_requested(false, false));
}

fn manager_with_active_resize() -> (WidgetManager, WidgetProfile, Rect, eframe::egui::Pos2) {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::WarframeMarket;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    let anchor = pos2(100.0, 120.0);
    profile.settings_mut(id).position = WidgetPosition { x: 0.8, y: 0.6 };

    manager.finish_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(320.0, 200.0),
        anchor,
        false,
        false,
        ResizeGripOutcome {
            drag_delta: vec2(24.0, 16.0),
            dragging: true,
            drag_stopped: false,
            drag_cancelled: false,
        },
    );
    assert_eq!(
        manager.screen_position(id, INTERACTIVE, viewport, 24.0, &profile),
        anchor
    );

    (manager, profile, viewport, anchor)
}

#[test]
fn passive_mode_cancels_an_interrupted_resize() {
    let (mut manager, profile, viewport, anchor) = manager_with_active_resize();

    manager.sync_interaction_state(PASSIVE, true, true);

    assert_ne!(
        manager.screen_position(WidgetId::WarframeMarket, PASSIVE, viewport, 24.0, &profile,),
        anchor
    );
}

#[test]
fn released_pointer_cancels_an_interrupted_resize() {
    let (mut manager, profile, viewport, anchor) = manager_with_active_resize();

    manager.sync_interaction_state(INTERACTIVE, true, false);

    assert_ne!(
        manager.screen_position(
            WidgetId::WarframeMarket,
            INTERACTIVE,
            viewport,
            24.0,
            &profile,
        ),
        anchor
    );
}

#[test]
fn missing_active_game_cancels_an_interrupted_resize() {
    let (mut manager, profile, viewport, anchor) = manager_with_active_resize();

    manager.sync_interaction_state(INTERACTIVE, false, true);

    assert_ne!(
        manager.screen_position(
            WidgetId::WarframeMarket,
            INTERACTIVE,
            viewport,
            24.0,
            &profile,
        ),
        anchor
    );
}

#[test]
fn valid_interaction_keeps_an_active_resize() {
    let (mut manager, profile, viewport, anchor) = manager_with_active_resize();

    manager.sync_interaction_state(INTERACTIVE, true, true);

    assert_eq!(
        manager.screen_position(
            WidgetId::WarframeMarket,
            INTERACTIVE,
            viewport,
            24.0,
            &profile,
        ),
        anchor
    );
}

#[test]
fn an_unrendered_resize_owner_is_cancelled_while_the_pointer_stays_down() {
    let (mut manager, _, _, _) = manager_with_active_resize();

    manager.begin_widget_frame();
    manager.sync_interaction_state(INTERACTIVE, true, true);

    assert!(manager.resize.is_none());
}

#[test]
fn a_completed_resize_is_persisted_before_the_widget_can_disappear() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::Notes;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    let anchor = pos2(300.0, 240.0);

    manager.finish_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(360.0, 280.0),
        anchor,
        false,
        false,
        ResizeGripOutcome {
            drag_delta: vec2(60.0, 40.0),
            dragging: true,
            drag_stopped: false,
            drag_cancelled: false,
        },
    );
    let save_requested = manager.finish_resizable_panel(
        id,
        INTERACTIVE,
        viewport,
        24.0,
        &mut profile,
        vec2(360.0, 280.0),
        anchor,
        false,
        false,
        ResizeGripOutcome {
            drag_delta: vec2(0.0, 0.0),
            dragging: false,
            drag_stopped: true,
            drag_cancelled: false,
        },
    );

    assert!(save_requested);
    assert_eq!(profile.notes.width, 420.0);
    assert_eq!(profile.notes.height, 320.0);
    manager.begin_widget_frame();
    manager.sync_interaction_state(PASSIVE, true, false);
    assert!(manager.resize.is_none());
    assert_eq!(profile.notes.width, 420.0);
    assert_eq!(profile.notes.height, 320.0);
    assert!(
        WidgetManager::default()
            .screen_position(id, INTERACTIVE, viewport, 24.0, &profile)
            .distance(anchor)
            <= 0.01
    );
}

#[test]
fn common_drag_policy_is_interactive_and_game_scoped() {
    assert!(widget_draggable(INTERACTIVE, true));
    assert!(!widget_draggable(PASSIVE, true));
    assert!(!widget_draggable(INTERACTIVE, false));
}

#[test]
fn enabled_notes_widget_renders_a_measured_panel() {
    let context = eframe::egui::Context::default();
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let mut notes = NotesWidgetState::default();
    profile.notes.enabled = true;
    notes.apply_update(NotesUpdate {
        document: crate::notes::NotesDocument::default(),
        save_pending: false,
        error: None,
        durability_warning: false,
    });
    let snapshot = CoreSnapshot {
        active_game: Some(GameWindow {
            pid: Some(42),
            steam_app_id: Some(230_410),
            app_id: Some("warframe.x64.exe".to_owned()),
            title: "Warframe".to_owned(),
            rect: GameRect {
                x: 0,
                y: 0,
                width: 1_920,
                height: 1_080,
            },
            scale: 1.0,
            backend: "test".to_owned(),
        }),
        overlay_mode: INTERACTIVE,
        ..CoreSnapshot::default()
    };
    let input = eframe::egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0))),
        ..Default::default()
    };

    let _ = context.run_ui(input, |ui| {
        manager.render_notes(ui, &snapshot, &mut notes, &mut profile, 24.0);
    });

    let measured = manager.measured_size(WidgetId::Notes, INTERACTIVE);
    assert!(measured.x > 1.0);
    assert!(measured.y > 1.0);
}

#[test]
fn joined_twitch_real_pointer_resize_survives_the_application_sync_order() {
    let context = eframe::egui::Context::default();
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let mut twitch = TwitchWidgetState::default();
    let prefs = TwitchPrefs {
        active_channel: Some("playervox".to_owned()),
        ..TwitchPrefs::default()
    };
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(936.0, 748.0));
    let margin = 24.0;
    profile.twitch_chat.enabled = true;
    profile.twitch_chat.width = 419.3;
    profile.twitch_chat.height = 357.7;
    profile.twitch_chat.position = WidgetPosition { x: 0.0, y: 0.0 };
    twitch.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            channel: Some("playervox".to_owned()),
            connection: TwitchConnectionState::Joined,
            authenticated_login: Some("viewer".to_owned()),
            credentials_available: true,
            client_configured: true,
            ..TwitchSnapshot::default()
        }),
    );
    let snapshot = CoreSnapshot {
        active_game: Some(GameWindow {
            pid: Some(42),
            steam_app_id: Some(230_410),
            app_id: Some("warframe.x64.exe".to_owned()),
            title: "Warframe".to_owned(),
            rect: GameRect {
                x: 0,
                y: 0,
                width: 936,
                height: 748,
            },
            scale: 1.25,
            backend: "test".to_owned(),
        }),
        overlay_mode: INTERACTIVE,
        ..CoreSnapshot::default()
    };

    let run_frame = |events: Vec<Event>,
                     manager: &mut WidgetManager,
                     profile: &mut WidgetProfile,
                     twitch: &mut TwitchWidgetState| {
        let save_requested = std::cell::Cell::new(false);
        let _ = context.run_ui(
            RawInput {
                screen_rect: Some(viewport),
                events,
                ..RawInput::default()
            },
            |ui| {
                manager.begin_widget_frame();
                save_requested.set(
                    manager
                        .render_twitch(
                            ui,
                            &snapshot,
                            twitch,
                            &prefs,
                            profile,
                            Instant::now(),
                            margin,
                        )
                        .save_requested,
                );
                let pointer_down = ui.input(|input| input.pointer.primary_down());
                manager.sync_interaction_state(
                    snapshot.overlay_mode,
                    snapshot.active_game.is_some(),
                    pointer_down,
                );
            },
        );
        save_requested.get()
    };

    run_frame(Vec::new(), &mut manager, &mut profile, &mut twitch);
    run_frame(Vec::new(), &mut manager, &mut profile, &mut twitch);
    let initial_top_left = manager.screen_position(
        WidgetId::TwitchChat,
        INTERACTIVE,
        viewport,
        margin,
        &profile,
    );
    let measured = manager.measured_size(WidgetId::TwitchChat, INTERACTIVE);
    let grip = initial_top_left + measured - vec2(8.0, 8.0);
    assert!(!run_frame(
        vec![
            Event::PointerMoved(grip),
            Event::PointerButton {
                pos: grip,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            Event::PointerMoved(grip + vec2(30.0, 20.0)),
        ],
        &mut manager,
        &mut profile,
        &mut twitch,
    ));
    assert!(!run_frame(
        vec![Event::PointerMoved(grip + vec2(60.0, 40.0))],
        &mut manager,
        &mut profile,
        &mut twitch,
    ));
    assert_eq!(
        manager.screen_position(
            WidgetId::TwitchChat,
            INTERACTIVE,
            viewport,
            margin,
            &profile,
        ),
        initial_top_left
    );

    let resize = manager.resize.expect("Twitch resize session");
    assert_eq!(resize.size, vec2(479.3, 397.7));
    assert_eq!(
        manager.screen_position(
            WidgetId::TwitchChat,
            INTERACTIVE,
            viewport,
            margin,
            &profile,
        ),
        initial_top_left
    );

    assert!(run_frame(
        vec![Event::PointerButton {
            pos: grip + vec2(90.0, 60.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
        &mut manager,
        &mut profile,
        &mut twitch,
    ));
    assert_eq!(profile.twitch_chat.width, 509.3);
    assert_eq!(profile.twitch_chat.height, 417.7);
    assert!(!run_frame(
        Vec::new(),
        &mut manager,
        &mut profile,
        &mut twitch,
    ));
    assert_eq!(
        manager.screen_position(
            WidgetId::TwitchChat,
            INTERACTIVE,
            viewport,
            margin,
            &profile,
        ),
        initial_top_left
    );
}

mod manual_stopwatch_integration_tests {
    use eframe::egui::{Rect, pos2, vec2};
    use overcrow_config::{WidgetId, WidgetPosition, WidgetProfile};

    use super::{INTERACTIVE, WidgetManager};

    #[test]
    fn manual_stopwatch_uses_common_measurement_position_and_drag_stop_policy() {
        let mut manager = WidgetManager::default();
        let mut profile = WidgetProfile::default();
        let viewport = Rect::from_min_size(pos2(100.0, 200.0), vec2(800.0, 600.0));
        let size = vec2(200.0, 100.0);
        let margin = 24.0;
        let initial_position = profile.manual_stopwatch.position;

        let save_while_dragging = manager.finish_drag_only(
            WidgetId::ManualStopwatch,
            INTERACTIVE,
            viewport,
            margin,
            &mut profile,
            size,
            pos2(400.0, 450.0),
            true,
            false,
        );

        assert_eq!(
            manager.measured_size(WidgetId::ManualStopwatch, INTERACTIVE,),
            size
        );
        assert_eq!(profile.manual_stopwatch.position, initial_position);
        assert!(!save_while_dragging);

        let save_after_release = manager.finish_drag_only(
            WidgetId::ManualStopwatch,
            INTERACTIVE,
            viewport,
            margin,
            &mut profile,
            size,
            pos2(400.0, 450.0),
            false,
            true,
        );

        assert_eq!(
            profile.manual_stopwatch.position,
            WidgetPosition { x: 0.5, y: 0.5 }
        );
        assert!(save_after_release);
    }
}
