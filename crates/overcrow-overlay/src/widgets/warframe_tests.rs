use eframe::egui::{
    self, Event, RawInput, Rect,
    accesskit::{Action as AccessKitAction, ActionRequest, NodeId, Role, TreeId},
    pos2, vec2,
};
use overcrow_config::{FissureEra, WarframePrefs};
use overcrow_protocol::OverlayMode;

use crate::warframe::{
    ActivityMission, ArchonHunt, FissureMission, InvasionMission, MarketItemSummary,
    MarketSnapshot, SortieMission, WarframeDerivedCache, WorldstateSnapshot,
};

use super::{
    warframe_fissures::paint_warframe_fissures, warframe_invasions::paint_warframe_invasions,
    warframe_market::paint_warframe_market, warframe_sortie::paint_warframe_sortie,
    warframe_status::paint_warframe_status,
};

fn raw_input(events: Vec<Event>) -> RawInput {
    raw_input_at_height(events, 600.0)
}

fn raw_input_at_height(events: Vec<Event>, height: f32) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, height))),
        events,
        ..RawInput::default()
    }
}

fn paint_at_height(height: f32, mut paint: impl FnMut(&mut egui::Ui) -> egui::Vec2) -> egui::Vec2 {
    let context = egui::Context::default();
    let mut size = egui::Vec2::ZERO;
    let _ = context.run_ui(raw_input_at_height(Vec::new(), height), |ui| {
        size = paint(ui);
    });
    size
}

#[allow(clippy::too_many_arguments)]
fn paint_test_warframe_invasions(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    panel_size: egui::Vec2,
    invasions: &[InvasionMission],
    prefs: &WarframePrefs,
    scale: f32,
    mode: OverlayMode,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> super::warframe_invasions::WarframeInvasionsResponse {
    let indices = (0..invasions.len()).collect::<Vec<_>>();
    let mut cache = WarframeDerivedCache::default();
    paint_warframe_invasions(
        ui,
        current_position,
        panel_size,
        invasions,
        &indices,
        &mut cache,
        1,
        1,
        prefs,
        scale,
        mode,
        transparent_background,
        draggable,
        true,
        margin,
    )
}

fn click_checkbox<Action>(
    checkbox_index: usize,
    mut paint: impl FnMut(&mut egui::Ui) -> Vec<Action>,
) -> Vec<Action> {
    let context = egui::Context::default();
    context.enable_accesskit();

    let mut actions = Vec::new();
    let output = context.run_ui(raw_input(Vec::new()), |ui| actions.extend(paint(ui)));
    assert!(actions.is_empty());
    let checkbox_id = output
        .platform_output
        .accesskit_update
        .expect("accessibility tree")
        .nodes
        .into_iter()
        .filter_map(|(id, node)| (node.role() == Role::CheckBox).then_some(id))
        .nth(checkbox_index)
        .expect("completion checkbox");
    let click = Event::AccessKitActionRequest(ActionRequest {
        action: AccessKitAction::Click,
        target_tree: TreeId::ROOT,
        target_node: checkbox_id,
        data: None,
    });

    let _ = context.run_ui(raw_input(vec![click]), |ui| actions.extend(paint(ui)));
    actions
}

fn sortie_snapshot() -> WorldstateSnapshot {
    WorldstateSnapshot {
        sortie: Some(SortieMission {
            boss: "Test Boss".to_owned(),
            expires_at_secs: 1_000,
            missions: vec![ActivityMission {
                mission_type: "Extermination".to_owned(),
                node: "SolNode1".to_owned(),
                modifier: Some("Enemy Elemental Enhancement".to_owned()),
            }],
        }),
        ..WorldstateSnapshot::default()
    }
}

#[test]
fn passive_sortie_block_completion_checkbox_emits_no_action() {
    let snapshot = sortie_snapshot();
    let prefs = WarframePrefs::default();

    let actions = click_checkbox(0, |ui| {
        paint_warframe_sortie(
            ui,
            pos2(8.0, 8.0),
            vec2(420.0, 300.0),
            &snapshot,
            &prefs,
            1.0,
            OverlayMode::Passive,
            0,
            false,
            false,
            true,
            0.0,
        )
        .actions
    });

    assert!(actions.is_empty());
}

#[test]
fn passive_sortie_mission_completion_checkbox_emits_no_action() {
    let snapshot = sortie_snapshot();
    let prefs = WarframePrefs::default();

    let actions = click_checkbox(1, |ui| {
        paint_warframe_sortie(
            ui,
            pos2(8.0, 8.0),
            vec2(420.0, 300.0),
            &snapshot,
            &prefs,
            1.0,
            OverlayMode::Passive,
            0,
            false,
            false,
            true,
            0.0,
        )
        .actions
    });

    assert!(actions.is_empty());
}

fn invasion() -> InvasionMission {
    invasion_with_instance("test-invasion", "SolNode1")
}

fn invasion_with_instance(instance_id: &str, node: &str) -> InvasionMission {
    InvasionMission {
        instance_id: instance_id.to_owned(),
        node: node.to_owned(),
        attacker_faction: "Grineer".to_owned(),
        defender_faction: "Corpus".to_owned(),
        attacker_reward: None,
        defender_reward: None,
        count: 10,
        goal: 100,
        completed: false,
    }
}

fn populated_worldstate() -> WorldstateSnapshot {
    let mission = ActivityMission {
        mission_type: "Extermination".to_owned(),
        node: "SolNode1".to_owned(),
        modifier: Some("Enemy Elemental Enhancement".to_owned()),
    };
    WorldstateSnapshot {
        sortie: Some(SortieMission {
            boss: "Test Boss".to_owned(),
            expires_at_secs: 10_000,
            missions: vec![mission.clone(); 3],
        }),
        archon: Some(ArchonHunt {
            boss: "Archon Boreal".to_owned(),
            expires_at_secs: 10_000,
            missions: vec![mission; 3],
        }),
        ..WorldstateSnapshot::default()
    }
}

fn warframe_panel_sizes(
    mode: OverlayMode,
    viewport_height: f32,
    populated: bool,
) -> [egui::Vec2; 4] {
    let panel_size = vec2(420.0, 300.0);
    let prefs = WarframePrefs::default();
    let fissures = if populated {
        (0..24)
            .map(|index| FissureMission {
                era: FissureEra::Lith,
                mission_type: "Extermination".to_owned(),
                node: format!("SolNode{index}"),
                expires_at_secs: 10_000,
                steel_path: false,
                is_storm: false,
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let fissure_indices = (0..fissures.len()).collect::<Vec<_>>();
    let market = MarketSnapshot {
        results: if populated {
            (0..12)
                .map(|index| MarketItemSummary {
                    name: format!("Test item {index}"),
                    slug: format!("test-item-{index}"),
                })
                .collect()
        } else {
            Vec::new()
        },
        ..MarketSnapshot::default()
    };
    let worldstate = populated.then(populated_worldstate).unwrap_or_default();
    let invasions = if populated {
        (0..12)
            .map(|index| invasion_with_instance(&format!("test-invasion-{index}"), "SolNode1"))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let fissures_size = paint_at_height(viewport_height, |ui| {
        paint_warframe_fissures(
            ui,
            pos2(24.0, 24.0),
            panel_size,
            &fissures,
            &fissure_indices,
            &prefs,
            1.0,
            mode,
            0,
            false,
            false,
            true,
            24.0,
        )
        .size
    });
    let mut query = String::new();
    let market_size = paint_at_height(viewport_height, |ui| {
        paint_warframe_market(
            ui,
            pos2(24.0, 24.0),
            panel_size,
            &market,
            &mut query,
            None,
            1.0,
            mode,
            false,
            false,
            true,
            24.0,
        )
        .size
    });
    let sortie_size = paint_at_height(viewport_height, |ui| {
        paint_warframe_sortie(
            ui,
            pos2(24.0, 24.0),
            panel_size,
            &worldstate,
            &prefs,
            1.0,
            mode,
            0,
            false,
            false,
            true,
            24.0,
        )
        .size
    });
    let invasion_indices = (0..invasions.len()).collect::<Vec<_>>();
    let mut cache = WarframeDerivedCache::default();
    let invasions_size = paint_at_height(viewport_height, |ui| {
        paint_warframe_invasions(
            ui,
            pos2(24.0, 24.0),
            panel_size,
            &invasions,
            &invasion_indices,
            &mut cache,
            1,
            1,
            &prefs,
            1.0,
            mode,
            false,
            false,
            true,
            24.0,
        )
        .size
    });

    [fissures_size, market_size, sortie_size, invasions_size]
}

#[test]
fn passive_warframe_panels_fit_short_content() {
    for size in warframe_panel_sizes(OverlayMode::Passive, 600.0, false) {
        assert_eq!(size.x, 420.0);
        assert!(size.y > 1.0);
        assert!(
            size.y < 300.0,
            "passive panel kept configured height: {size:?}"
        );
    }
}

#[test]
fn passive_warframe_panels_cap_long_content_to_the_viewport() {
    for size in warframe_panel_sizes(OverlayMode::Passive, 180.0, true) {
        assert!(size.y <= 180.0, "passive panel exceeded viewport: {size:?}");
    }
}

#[test]
fn interactive_warframe_panels_do_not_exceed_configured_height() {
    for (name, size) in ["fissures", "market", "sortie", "invasions"]
        .into_iter()
        .zip(warframe_panel_sizes(OverlayMode::Interactive, 600.0, true))
    {
        assert!(size.y <= 300.0, "{name} panel exceeded profile: {size:?}");
    }
}

fn passive_invasion_completion_actions(prefs: &WarframePrefs) -> Vec<super::InvasionPrefsAction> {
    let invasions = vec![invasion()];

    click_checkbox(0, |ui| {
        paint_test_warframe_invasions(
            ui,
            pos2(8.0, 8.0),
            vec2(420.0, 300.0),
            &invasions,
            prefs,
            1.0,
            OverlayMode::Passive,
            false,
            false,
            0.0,
        )
        .actions
    })
}

#[test]
fn passive_normal_invasion_completion_checkbox_emits_no_action() {
    let prefs = WarframePrefs::default();

    assert!(passive_invasion_completion_actions(&prefs).is_empty());
}

#[test]
fn passive_compact_invasion_completion_checkbox_emits_no_action() {
    let prefs = WarframePrefs {
        invasion_compact: true,
        ..WarframePrefs::default()
    };

    assert!(passive_invasion_completion_actions(&prefs).is_empty());
}

#[test]
fn interactive_invasion_completion_checkbox_remains_actionable() {
    let invasions = vec![invasion()];
    let prefs = WarframePrefs::default();

    let actions = click_checkbox(0, |ui| {
        paint_test_warframe_invasions(
            ui,
            pos2(8.0, 8.0),
            vec2(420.0, 300.0),
            &invasions,
            &prefs,
            1.0,
            OverlayMode::Interactive,
            false,
            false,
            0.0,
        )
        .actions
    });

    assert_eq!(actions.len(), 1);
}

#[test]
fn same_node_invasion_checkboxes_emit_instance_scoped_completion_keys() {
    let invasions = vec![
        invasion_with_instance("provider-object-a", "SolNode1"),
        invasion_with_instance("provider-object-b", "SolNode1"),
    ];
    let prefs = WarframePrefs::default();

    let actions = click_checkbox(1, |ui| {
        paint_test_warframe_invasions(
            ui,
            pos2(8.0, 8.0),
            vec2(420.0, 300.0),
            &invasions,
            &prefs,
            1.0,
            OverlayMode::Interactive,
            false,
            false,
            0.0,
        )
        .actions
    });

    assert_eq!(
        actions,
        vec![super::InvasionPrefsAction::ToggleDone(
            "invasion:provider-object-b".to_owned()
        )]
    );
}

fn compact_invasion_checkbox_ids(
    context: &egui::Context,
    invasions: &[InvasionMission],
) -> Vec<NodeId> {
    let prefs = WarframePrefs {
        invasion_compact: true,
        ..WarframePrefs::default()
    };
    context
        .run_ui(raw_input(Vec::new()), |ui| {
            paint_test_warframe_invasions(
                ui,
                pos2(8.0, 8.0),
                vec2(420.0, 300.0),
                invasions,
                &prefs,
                1.0,
                OverlayMode::Interactive,
                false,
                false,
                0.0,
            );
        })
        .platform_output
        .accesskit_update
        .expect("accessibility tree")
        .nodes
        .into_iter()
        .filter_map(|(id, node)| (node.role() == Role::CheckBox).then_some(id))
        .collect()
}

#[test]
fn compact_invasion_checkbox_ids_follow_instances_when_rows_reorder() {
    let instance_a_id = {
        let context = egui::Context::default();
        context.enable_accesskit();
        compact_invasion_checkbox_ids(
            &context,
            &[invasion_with_instance("provider-object-a", "SolNode1")],
        )[0]
    };
    let instance_b_id = {
        let context = egui::Context::default();
        context.enable_accesskit();
        compact_invasion_checkbox_ids(
            &context,
            &[invasion_with_instance("provider-object-b", "SolNode1")],
        )[0]
    };
    let context = egui::Context::default();
    context.enable_accesskit();
    let first = vec![
        invasion_with_instance("provider-object-a", "SolNode1"),
        invasion_with_instance("provider-object-b", "SolNode1"),
    ];
    let reversed = vec![first[1].clone(), first[0].clone()];

    let first_ids = compact_invasion_checkbox_ids(&context, &first);
    let reversed_ids = compact_invasion_checkbox_ids(&context, &reversed);

    assert_eq!(first_ids.len(), 2);
    assert_eq!(reversed_ids.len(), 2);
    assert_ne!(instance_a_id, instance_b_id);
    assert!(first_ids.contains(&instance_a_id));
    assert!(first_ids.contains(&instance_b_id));
    assert!(reversed_ids.contains(&instance_a_id));
    assert!(reversed_ids.contains(&instance_b_id));
}

#[test]
fn retained_worldstate_content_renders_alongside_the_refresh_error() {
    let context = egui::Context::default();
    context.enable_accesskit();
    let snapshot = WorldstateSnapshot {
        fetched_at_secs: 1_000,
        last_attempt_at_secs: 1_100,
        daily_reset_at_secs: Some(2_000),
        error: Some("refresh failed".to_owned()),
        ..WorldstateSnapshot::default()
    };

    let output = context.run_ui(raw_input(Vec::new()), |ui| {
        paint_warframe_status(
            ui,
            pos2(8.0, 8.0),
            vec2(420.0, 300.0),
            &snapshot,
            &WarframePrefs::default(),
            1.0,
            OverlayMode::Passive,
            1_100,
            false,
            false,
            true,
            0.0,
        );
    });
    let visible_text = output
        .platform_output
        .accesskit_update
        .expect("accessibility tree")
        .nodes
        .into_iter()
        .filter_map(|(_, node)| node.value().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(" | ");

    assert!(visible_text.contains("refresh failed"), "{visible_text}");
    assert!(visible_text.contains("Reset"), "{visible_text}");
    assert!(visible_text.contains("00:00 UTC"), "{visible_text}");
}
