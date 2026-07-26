use eframe::egui::{self, Color32, Layout, Vec2, vec2};
use overcrow_config::WarframePrefs;
use overcrow_protocol::OverlayMode;

use super::{
    WidgetGlyph,
    chrome::{
        ACCENT, BODY_SIZE, META_SIZE, ResizeGripOutcome, TIMER_SIZE, apply_scale, meta_text,
        panel_content_height, panel_content_width, panel_frame, panel_width_limits, resize_grip,
        scaled_content_font_size, status_pill, timer_color, widget_identity,
    },
};
use crate::warframe::{
    ActivityMission, ArchonHunt, SortieMission, WorldstateSnapshot, archon_mission_keys,
    archon_shard_hint, format_mission_type, format_node, format_remaining, sortie_mission_keys,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SortiePrefsAction {
    ToggleSeconds,
    ToggleDone(String),
    /// Mark every mission of a block done, or clear them all.
    SetBlockDone {
        keys: Vec<String>,
        done: bool,
    },
}

pub struct WarframeSortieResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub resize: ResizeGripOutcome,
    pub actions: Vec<SortiePrefsAction>,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_warframe_sortie(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    panel_size: Vec2,
    snapshot: &WorldstateSnapshot,
    prefs: &WarframePrefs,
    scale: f32,
    mode: OverlayMode,
    now_secs: u64,
    transparent_background: bool,
    draggable: bool,
    input_enabled: bool,
    margin: f32,
) -> WarframeSortieResponse {
    let panel_size = super::chrome::clamp_panel_size(panel_size);
    let mut resize = ResizeGripOutcome::default();
    let mut actions = Vec::new();

    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("warframe-sortie-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(input_enabled)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            if !input_enabled {
                ui.disable();
            }
            apply_scale(ui, scale);
            let frame = panel_frame(transparent_background).show(ui, |ui| {
                panel_width_limits(ui, panel_size.x, transparent_background);
                let height_limit = if mode == OverlayMode::Interactive {
                    panel_content_height(panel_size.y, transparent_background)
                } else {
                    panel_content_height(
                        (viewport.height() - margin * 2.0).max(1.0),
                        transparent_background,
                    )
                };
                ui.set_max_height(height_limit);

                paint_header(ui);
                ui.add_space(4.0);

                let header_h = 36.0 * scale;
                let body_max = (height_limit - header_h).max(64.0);

                if let Some(error) = snapshot.error.as_deref()
                    && snapshot.sortie.is_none()
                    && snapshot.archon.is_none()
                {
                    ui.label(meta_text(error));
                } else if snapshot.sortie.is_none() && snapshot.archon.is_none() {
                    ui.label(meta_text("No activity"));
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("sortie-scroll")
                        .max_height(body_max)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.set_min_width(
                                (panel_content_width(panel_size.x, transparent_background) - 32.0)
                                    .max(1.0),
                            );
                            if let Some(sortie) = &snapshot.sortie {
                                paint_sortie_block(ui, sortie, prefs, mode, now_secs, &mut actions);
                            }
                            if let Some(archon) = &snapshot.archon {
                                if snapshot.sortie.is_some() {
                                    ui.add_space(6.0);
                                    ui.separator();
                                    ui.add_space(6.0);
                                }
                                paint_archon_block(ui, archon, prefs, mode, now_secs, &mut actions);
                            }
                        });
                }
            });
            resize = resize_grip(
                ui,
                frame.response.rect,
                input_enabled && mode == OverlayMode::Interactive,
            );
        });

    let measured = response.response.rect.size().max(vec2(1.0, 1.0));
    WarframeSortieResponse {
        size: measured,
        position: response.response.rect.min,
        dragged: response.response.dragged() && !resize.dragging,
        drag_stopped: response.response.drag_stopped() && !resize.dragging && !resize.drag_stopped,
        resize,
        actions,
    }
}

fn paint_header(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        widget_identity(
            ui,
            WidgetGlyph::Missions,
            "SORTIE & ARCHON",
            Some("MISSION TRACKER"),
        );
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            status_pill(ui, "DAILY + WEEKLY", ACCENT);
        });
    });
}

pub(crate) fn paint_sortie_options(
    ui: &mut egui::Ui,
    prefs: &WarframePrefs,
    actions: &mut Vec<SortiePrefsAction>,
) {
    let mut seconds = prefs.show_activity_seconds;
    if ui.checkbox(&mut seconds, "Seconds").changed() {
        actions.push(SortiePrefsAction::ToggleSeconds);
    }
}

fn paint_sortie_block(
    ui: &mut egui::Ui,
    sortie: &SortieMission,
    prefs: &WarframePrefs,
    mode: OverlayMode,
    now_secs: u64,
    actions: &mut Vec<SortiePrefsAction>,
) {
    let keys = sortie_mission_keys(sortie.expires_at_secs, sortie.missions.len());
    let all_done = !keys.is_empty() && keys.iter().all(|key| prefs.activity_is_done(key));
    paint_section_title(
        ui,
        "SORTIE",
        &sortie.boss,
        None,
        format_remaining(
            now_secs,
            sortie.expires_at_secs,
            prefs.show_activity_seconds,
        ),
        all_done,
        keys.clone(),
        mode,
        actions,
    );
    if all_done {
        return;
    }
    for (index, mission) in sortie.missions.iter().enumerate() {
        let key = keys[index].clone();
        paint_mission_row(
            ui,
            mission,
            true,
            prefs.activity_is_done(&key),
            key,
            mode,
            actions,
        );
    }
}

fn paint_archon_block(
    ui: &mut egui::Ui,
    archon: &ArchonHunt,
    prefs: &WarframePrefs,
    mode: OverlayMode,
    now_secs: u64,
    actions: &mut Vec<SortiePrefsAction>,
) {
    let keys = archon_mission_keys(archon.expires_at_secs, archon.missions.len());
    let all_done = !keys.is_empty() && keys.iter().all(|key| prefs.activity_is_done(key));
    let shard = archon_shard_hint(&archon.boss);
    paint_section_title(
        ui,
        "ARCHON",
        &archon.boss,
        if all_done { None } else { shard },
        format_remaining(
            now_secs,
            archon.expires_at_secs,
            prefs.show_activity_seconds,
        ),
        all_done,
        keys.clone(),
        mode,
        actions,
    );
    if all_done {
        return;
    }
    for (index, mission) in archon.missions.iter().enumerate() {
        let key = keys[index].clone();
        paint_mission_row(
            ui,
            mission,
            false,
            prefs.activity_is_done(&key),
            key,
            mode,
            actions,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_section_title(
    ui: &mut egui::Ui,
    kind: &str,
    boss: &str,
    shard: Option<crate::warframe::ArchonShardHint>,
    remaining: String,
    all_done: bool,
    keys: Vec<String>,
    mode: OverlayMode,
    actions: &mut Vec<SortiePrefsAction>,
) {
    ui.horizontal(|ui| {
        let mut checked = all_done;
        if ui
            .add_enabled(
                mode == OverlayMode::Interactive,
                egui::Checkbox::new(&mut checked, ""),
            )
            .on_hover_text(if all_done {
                "Mark block incomplete"
            } else {
                "Mark all missions done"
            })
            .changed()
        {
            actions.push(SortiePrefsAction::SetBlockDone {
                keys,
                done: checked,
            });
        }
        let kind_color = if all_done {
            Color32::from_gray(140)
        } else {
            Color32::from_rgb(255, 190, 100)
        };
        let boss_color = if all_done {
            Color32::from_gray(140)
        } else {
            Color32::from_gray(220)
        };
        let mut kind_text = egui::RichText::new(kind)
            .size(scaled_content_font_size(ui, BODY_SIZE))
            .color(kind_color);
        let mut boss_text = egui::RichText::new(boss)
            .size(scaled_content_font_size(ui, BODY_SIZE))
            .color(boss_color);
        if all_done {
            kind_text = kind_text.strikethrough();
            boss_text = boss_text.strikethrough();
        } else {
            kind_text = kind_text.strong();
            boss_text = boss_text.strong();
        }
        ui.label(kind_text);
        ui.label(boss_text);
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(remaining)
                    .monospace()
                    .size(scaled_content_font_size(ui, TIMER_SIZE - 1.0))
                    .strong()
                    .color(if all_done {
                        Color32::from_gray(140)
                    } else {
                        timer_color()
                    }),
            );
        });
    });
    if let Some(shard) = shard {
        ui.horizontal(|ui| {
            let color = Color32::from_rgb(shard.r, shard.g, shard.b);
            let (dot_rect, _) =
                ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(dot_rect.center(), 4.5, color);
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(shard.label)
                    .size(scaled_content_font_size(ui, META_SIZE))
                    .strong()
                    .color(color),
            );
        });
    }
    if !all_done {
        ui.add_space(2.0);
    }
}

fn paint_mission_row(
    ui: &mut egui::Ui,
    mission: &ActivityMission,
    show_modifier: bool,
    done: bool,
    key: String,
    mode: OverlayMode,
    actions: &mut Vec<SortiePrefsAction>,
) {
    ui.horizontal(|ui| {
        let mut checked = done;
        if ui
            .add_enabled(
                mode == OverlayMode::Interactive,
                egui::Checkbox::new(&mut checked, ""),
            )
            .changed()
        {
            actions.push(SortiePrefsAction::ToggleDone(key));
        }
        let mission_color = if done {
            Color32::from_gray(140)
        } else {
            Color32::from_gray(215)
        };
        let mut mission_text = egui::RichText::new(format_mission_type(&mission.mission_type))
            .size(scaled_content_font_size(ui, BODY_SIZE))
            .color(mission_color);
        if done {
            mission_text = mission_text.strikethrough();
        } else {
            mission_text = mission_text.strong();
        }
        ui.label(mission_text);
        ui.label(
            egui::RichText::new(format_node(&mission.node))
                .size(scaled_content_font_size(ui, BODY_SIZE - 1.0))
                .color(if done {
                    Color32::from_gray(120)
                } else {
                    Color32::from_gray(195)
                }),
        );
        if show_modifier && let Some(modifier) = mission.modifier.as_deref() {
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(modifier)
                        .size(scaled_content_font_size(ui, META_SIZE))
                        .color(Color32::from_rgb(170, 200, 255)),
                );
            });
        }
    });
}

pub fn apply_sortie_prefs_action(prefs: &mut WarframePrefs, action: SortiePrefsAction) {
    match action {
        SortiePrefsAction::ToggleSeconds => {
            prefs.show_activity_seconds = !prefs.show_activity_seconds;
        }
        SortiePrefsAction::ToggleDone(key) => {
            prefs.toggle_activity_done(&key);
        }
        SortiePrefsAction::SetBlockDone { keys, done } => {
            for key in keys {
                prefs.set_activity_done(&key, done);
            }
        }
    }
}
