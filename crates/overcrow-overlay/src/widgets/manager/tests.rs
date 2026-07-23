use std::collections::BTreeSet;

use eframe::egui::{Rect, pos2, vec2};
use overcrow_config::{WidgetId, WidgetPosition, WidgetProfile};
use overcrow_protocol::OverlayMode;

use super::{WidgetManager, placement_save_requested, widget_draggable, widget_visible};
use crate::widgets::{BUILTIN_WIDGETS, WidgetDescriptor, chrome::ResizeGripOutcome};

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

    assert!(!widget_visible(
        WidgetId::Clock,
        OverlayMode::Passive,
        true,
        &profile
    ));

    profile.clock.show_in_passive = true;

    assert!(widget_visible(
        WidgetId::Clock,
        OverlayMode::Passive,
        true,
        &profile
    ));
}

#[test]
fn visibility_requires_an_enabled_widget_and_active_game() {
    let mut profile = WidgetProfile::default();

    assert!(!widget_visible(
        WidgetId::Clock,
        OverlayMode::Interactive,
        true,
        &profile
    ));

    profile.clock.enabled = true;

    assert!(widget_visible(
        WidgetId::Clock,
        OverlayMode::Interactive,
        true,
        &profile
    ));
    assert!(!widget_visible(
        WidgetId::Clock,
        OverlayMode::Interactive,
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
        manager.set_measured_size(id, widget_size);
    }

    for viewport in [
        Rect::from_min_size(pos2(100.0, 200.0), vec2(1_920.0, 1_080.0)),
        Rect::from_min_size(pos2(40.0, 60.0), vec2(800.0, 600.0)),
    ] {
        for id in WidgetId::ALL {
            let top_left = manager.screen_position(id, viewport, margin, &profile);

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
        manager.set_measured_size(id, widget_size);
    }
    let positions = ids.map(|id| manager.screen_position(id, viewport, margin, &profile));

    assert_ne!(positions[0], positions[1]);
    assert_ne!(positions[0], positions[2]);
    assert_ne!(positions[1], positions[2]);
}

#[test]
fn manager_keeps_measured_sizes_and_catalog_state_transient() {
    let mut manager = WidgetManager::default();

    assert_eq!(
        manager.measured_size(WidgetId::Media),
        eframe::egui::Vec2::ZERO
    );
    assert!(!manager.catalog_open());

    manager.set_measured_size(WidgetId::Media, vec2(320.0, 140.0));
    manager.set_catalog_open(true);

    assert_eq!(manager.measured_size(WidgetId::Media), vec2(320.0, 140.0));
    assert!(manager.catalog_open());
}

#[test]
fn measured_content_height_does_not_replace_stored_panel_height() {
    let mut manager = WidgetManager::default();
    let mut profile = WidgetProfile::default();
    let id = WidgetId::WarframeMarket;
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0));
    profile.settings_mut(id).height = 400.0;

    let save_requested = manager.finish_warframe_panel(
        id,
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
    assert_eq!(manager.measured_size(id), vec2(320.0, 100.0));
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
        manager.set_measured_size(id, vec2(320.0, 160.0));

        manager.finish_warframe_panel(
            id,
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
            },
        );

        let save_requested = manager.finish_warframe_panel(
            id,
            viewport,
            margin,
            &mut profile,
            rendered_size,
            visible_top_left,
            false,
            false,
            ResizeGripOutcome {
                drag_delta: vec2(40.0, 80.0),
                dragging: false,
                drag_stopped: true,
            },
        );

        assert!(save_requested);
        assert_eq!(profile.settings(id).width, 360.0);
        assert_eq!(profile.settings(id).height, 480.0);
        assert_eq!(manager.measured_size(id), rendered_size);
        let next = manager.screen_position(id, viewport, margin, &profile);
        assert!((next.x - visible_top_left.x).abs() < 0.01);
        assert!((next.y - visible_top_left.y).abs() < 0.01);
    }
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

    manager.finish_warframe_panel(
        id,
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
        },
    );
    assert_eq!(
        manager.screen_position(id, viewport, 24.0, &profile),
        anchor
    );

    (manager, profile, viewport, anchor)
}

#[test]
fn passive_mode_cancels_an_interrupted_resize() {
    let (mut manager, profile, viewport, anchor) = manager_with_active_resize();

    manager.sync_interaction_state(OverlayMode::Passive, true, true);

    assert_ne!(
        manager.screen_position(WidgetId::WarframeMarket, viewport, 24.0, &profile),
        anchor
    );
}

#[test]
fn released_pointer_cancels_an_interrupted_resize() {
    let (mut manager, profile, viewport, anchor) = manager_with_active_resize();

    manager.sync_interaction_state(OverlayMode::Interactive, true, false);

    assert_ne!(
        manager.screen_position(WidgetId::WarframeMarket, viewport, 24.0, &profile),
        anchor
    );
}

#[test]
fn missing_active_game_cancels_an_interrupted_resize() {
    let (mut manager, profile, viewport, anchor) = manager_with_active_resize();

    manager.sync_interaction_state(OverlayMode::Interactive, false, true);

    assert_ne!(
        manager.screen_position(WidgetId::WarframeMarket, viewport, 24.0, &profile),
        anchor
    );
}

#[test]
fn valid_interaction_keeps_an_active_resize() {
    let (mut manager, profile, viewport, anchor) = manager_with_active_resize();

    manager.sync_interaction_state(OverlayMode::Interactive, true, true);

    assert_eq!(
        manager.screen_position(WidgetId::WarframeMarket, viewport, 24.0, &profile),
        anchor
    );
}

#[test]
fn common_drag_policy_is_interactive_and_game_scoped() {
    assert!(widget_draggable(OverlayMode::Interactive, true));
    assert!(!widget_draggable(OverlayMode::Passive, true));
    assert!(!widget_draggable(OverlayMode::Interactive, false));
}

mod manual_stopwatch_integration_tests {
    use eframe::egui::{Rect, pos2, vec2};
    use overcrow_config::{WidgetId, WidgetPosition, WidgetProfile};

    use super::WidgetManager;

    #[test]
    fn manual_stopwatch_uses_common_measurement_position_and_drag_stop_policy() {
        let mut manager = WidgetManager::default();
        let mut profile = WidgetProfile::default();
        let viewport = Rect::from_min_size(pos2(100.0, 200.0), vec2(800.0, 600.0));
        let size = vec2(200.0, 100.0);
        let margin = 24.0;

        let save_while_dragging = manager.finish_drag_only(
            WidgetId::ManualStopwatch,
            viewport,
            margin,
            &mut profile,
            size,
            pos2(400.0, 450.0),
            true,
            false,
        );

        assert_eq!(manager.measured_size(WidgetId::ManualStopwatch), size);
        assert_eq!(
            profile.manual_stopwatch.position,
            WidgetPosition { x: 0.5, y: 0.5 }
        );
        assert!(!save_while_dragging);

        let save_after_release = manager.finish_drag_only(
            WidgetId::ManualStopwatch,
            viewport,
            margin,
            &mut profile,
            size,
            pos2(676.0, 676.0),
            false,
            true,
        );

        assert_eq!(
            profile.manual_stopwatch.position,
            WidgetPosition { x: 1.0, y: 1.0 }
        );
        assert!(save_after_release);
    }
}
