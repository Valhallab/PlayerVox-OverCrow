use eframe::egui::{self, Color32, Layout, Vec2};
use overcrow_config::{FissureEra, FissureSource, WarframePrefs};
use overcrow_protocol::OverlayMode;

use super::chrome::{
    BODY_SIZE, META_SIZE, ResizeGripOutcome, TIMER_SIZE, apply_scale, era_color, meta_text,
    options_menu, panel_frame, resize_grip, timer_color, title_text,
};
use crate::warframe::{
    FissureMission, fissure_source, format_mission_type, format_node, format_remaining,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FissurePrefsAction {
    ToggleEra(FissureEra),
    ToggleSource(FissureSource),
    ToggleShowNode,
    ToggleSeconds,
}

pub struct WarframeFissuresResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub resize: ResizeGripOutcome,
    pub actions: Vec<FissurePrefsAction>,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_warframe_fissures(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    panel_size: Vec2,
    fissures: &[FissureMission],
    fissure_indices: &[usize],
    prefs: &WarframePrefs,
    scale: f32,
    mode: OverlayMode,
    now_secs: u64,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> WarframeFissuresResponse {
    let visible_count = fissure_indices
        .iter()
        .filter(|index| fissures[**index].expires_at_secs > now_secs)
        .count();
    let mut actions = Vec::new();
    let panel_size =
        super::chrome::clamp_panel_size_min(panel_size, super::chrome::FISSURE_PANEL_MIN_WIDTH);
    let mut resize = ResizeGripOutcome::default();

    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("warframe-fissures-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(true)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            apply_scale(ui, scale);
            panel_frame(transparent_background).show(ui, |ui| {
                ui.set_min_size(panel_size);
                ui.set_max_size(panel_size);

                if mode == OverlayMode::Interactive {
                    paint_fissure_header(ui, visible_count, prefs, &mut actions);
                    ui.add_space(4.0);
                } else {
                    ui.horizontal(|ui| {
                        ui.label(title_text("FISSURES"));
                        ui.label(
                            egui::RichText::new(format!("{visible_count}"))
                                .size(META_SIZE)
                                .strong()
                                .color(Color32::from_gray(180)),
                        );
                    });
                    ui.add_space(4.0);
                }

                let header_h = if mode == OverlayMode::Interactive {
                    72.0 * scale
                } else {
                    36.0 * scale
                };
                let list_height = (panel_size.y - header_h).max(90.0);

                if visible_count == 0 {
                    ui.label(meta_text("No matching fissures"));
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("fissure-scroll")
                        .max_height(list_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for source in FissureSource::ALL {
                                if !prefs.source_enabled(source) {
                                    continue;
                                }
                                let section_count = fissure_indices
                                    .iter()
                                    .filter(|index| {
                                        let fissure = &fissures[**index];
                                        fissure.expires_at_secs > now_secs
                                            && fissure_source(fissure) == source
                                    })
                                    .count();

                                let salt = match source {
                                    FissureSource::Normal => "section-normal",
                                    FissureSource::SteelPath => "section-steel",
                                    FissureSource::Railjack => "section-railjack",
                                };
                                egui::CollapsingHeader::new(
                                    egui::RichText::new(format!(
                                        "{}  ({})",
                                        source.label(),
                                        section_count
                                    ))
                                    .size(BODY_SIZE)
                                    .strong()
                                    .color(Color32::from_gray(220)),
                                )
                                .id_salt(salt)
                                .default_open(true)
                                .show(ui, |ui| {
                                    if section_count == 0 {
                                        ui.label(meta_text("—"));
                                        return;
                                    }
                                    for fissure in fissure_indices.iter().filter_map(|index| {
                                        let fissure = &fissures[*index];
                                        (fissure.expires_at_secs > now_secs
                                            && fissure_source(fissure) == source)
                                            .then_some(fissure)
                                    }) {
                                        paint_fissure_row(ui, fissure, prefs, now_secs);
                                    }
                                });
                            }
                        });
                }

                let panel_rect = ui.min_rect();
                // Resizable only while interactive — independent of Area move flag
                // (move is suppressed near the grip so the two do not fight).
                resize = resize_grip(ui, panel_rect, mode == OverlayMode::Interactive);
            });
        });

    WarframeFissuresResponse {
        size: panel_size,
        position: response.response.rect.min,
        dragged: response.response.dragged() && !resize.dragging,
        drag_stopped: response.response.drag_stopped() && !resize.dragging && !resize.drag_stopped,
        resize,
        actions,
    }
}

/// Title + count + **filters** (source / era); **options** (display) behind ⚙.
fn paint_fissure_header(
    ui: &mut egui::Ui,
    count: usize,
    prefs: &WarframePrefs,
    actions: &mut Vec<FissurePrefsAction>,
) {
    ui.horizontal(|ui| {
        ui.label(title_text("FISSURES"));
        ui.label(
            egui::RichText::new(format!("{count}"))
                .size(META_SIZE)
                .strong()
                .color(Color32::from_gray(180)),
        );
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            options_menu(ui, |ui| {
                let mut show_node = prefs.show_fissure_node;
                if ui.checkbox(&mut show_node, "Node / planet").changed() {
                    actions.push(FissurePrefsAction::ToggleShowNode);
                }
                let mut seconds = prefs.show_fissure_seconds;
                if ui.checkbox(&mut seconds, "Seconds").changed() {
                    actions.push(FissurePrefsAction::ToggleSeconds);
                }
            });
        });
    });
    ui.horizontal_wrapped(|ui| {
        for source in FissureSource::ALL {
            let mut enabled = prefs.source_enabled(source);
            if ui
                .checkbox(
                    &mut enabled,
                    egui::RichText::new(source.label()).size(META_SIZE),
                )
                .changed()
            {
                actions.push(FissurePrefsAction::ToggleSource(source));
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        for era in FissureEra::ALL {
            let mut enabled = prefs.era_enabled(era)
                && (prefs.fissure_eras.is_empty() || prefs.fissure_eras.contains(&era));
            if prefs.fissure_eras.is_empty() {
                enabled = true;
            }
            let label = egui::RichText::new(era.label())
                .size(META_SIZE)
                .color(era_color(era.label()));
            if ui.checkbox(&mut enabled, label).changed() {
                actions.push(FissurePrefsAction::ToggleEra(era));
            }
        }
    });
}

/// Natural single-line row: left cluster + timer pushed to the trailing edge.
fn paint_fissure_row(
    ui: &mut egui::Ui,
    fissure: &FissureMission,
    prefs: &WarframePrefs,
    now_secs: u64,
) {
    let era = fissure.era.label();
    let mission = format_mission_type(&fissure.mission_type);
    let remaining = format_remaining(
        now_secs,
        fissure.expires_at_secs,
        prefs.show_fissure_seconds,
    );

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(era)
                .size(BODY_SIZE)
                .strong()
                .color(era_color(era)),
        );
        ui.label(
            egui::RichText::new(mission)
                .size(BODY_SIZE)
                .color(Color32::from_gray(215)),
        );
        if prefs.show_fissure_node {
            ui.label(
                egui::RichText::new(format_node(&fissure.node))
                    .size(BODY_SIZE - 1.0)
                    .color(Color32::from_gray(195)),
            );
        }

        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(remaining)
                    .monospace()
                    .size(TIMER_SIZE - 1.0)
                    .strong()
                    .color(timer_color()),
            );
        });
    });
}

pub fn apply_fissure_prefs_action(prefs: &mut WarframePrefs, action: FissurePrefsAction) {
    match action {
        FissurePrefsAction::ToggleSource(source) => {
            let next = !prefs.source_enabled(source);
            let others_on = FissureSource::ALL
                .into_iter()
                .filter(|s| *s != source)
                .any(|s| prefs.source_enabled(s));
            if !next && !others_on {
                return;
            }
            match source {
                FissureSource::Normal => prefs.show_normal = next,
                FissureSource::SteelPath => prefs.show_steel_path = next,
                FissureSource::Railjack => prefs.show_railjack = next,
            }
        }
        FissurePrefsAction::ToggleEra(era) => {
            if prefs.fissure_eras.is_empty() {
                prefs.fissure_eras = FissureEra::ALL
                    .into_iter()
                    .filter(|candidate| *candidate != era)
                    .collect();
            } else if let Some(index) = prefs.fissure_eras.iter().position(|item| *item == era) {
                prefs.fissure_eras.remove(index);
            } else {
                prefs.fissure_eras.push(era);
            }
            if prefs.fissure_eras.len() == FissureEra::ALL.len() {
                prefs.fissure_eras.clear();
            }
        }
        FissurePrefsAction::ToggleShowNode => {
            prefs.show_fissure_node = !prefs.show_fissure_node;
        }
        FissurePrefsAction::ToggleSeconds => {
            prefs.show_fissure_seconds = !prefs.show_fissure_seconds;
        }
    }
}
