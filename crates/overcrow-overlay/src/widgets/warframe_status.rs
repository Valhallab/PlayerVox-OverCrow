use eframe::egui::{self, Color32, Layout, Sense, Stroke, Vec2, vec2};
use overcrow_config::{StatusRow, WarframePrefs};
use overcrow_protocol::OverlayMode;

use super::{
    WidgetGlyph,
    chrome::{
        ACCENT, BODY_SIZE, META_SIZE, ResizeGripOutcome, TEXT_PRIMARY, TIMER_SIZE, accent_error,
        accent_ok, accent_warn, apply_scale, current_content_scale, cycle_state_color, filter_chip,
        meta_text, panel_content_width, panel_frame, resize_grip, scaled_content_font_size,
        timer_color, widget_identity,
    },
};
use crate::warframe::{WorldstateSnapshot, format_remaining};

/// Horizontal bar type: planet/state dominant, timer secondary.
const BAR_TIMER_SIZE: f32 = 11.5;
const BAR_NAME_SIZE: f32 = 14.0;
const BAR_STATE_SIZE: f32 = 14.0;
const BAR_CELL_GAP: f32 = 1.0;
const BAR_SEP_PAD: f32 = 8.0;
/// Keep the editor usable when few rows are checked.
const HORIZONTAL_EDIT_MIN_WIDTH: f32 = 380.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusPrefsAction {
    ToggleHorizontal,
    ToggleSeconds,
    ToggleRow(StatusRow),
}

pub struct WarframeStatusResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub resize: ResizeGripOutcome,
    pub actions: Vec<StatusPrefsAction>,
}

struct StatusItem {
    name: String,
    state: String,
    state_color: Color32,
    time: String,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_warframe_status(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    panel_size: Vec2,
    snapshot: &WorldstateSnapshot,
    prefs: &WarframePrefs,
    scale: f32,
    mode: OverlayMode,
    now_secs: u64,
    transparent_background: bool,
    interactive: bool,
    input_enabled: bool,
    margin: f32,
) -> WarframeStatusResponse {
    let panel_size = super::chrome::clamp_panel_size(panel_size);
    let mut resize = ResizeGripOutcome::default();
    let mut actions = Vec::new();
    let items = collect_status_items(snapshot, prefs, now_secs);
    let horizontal = prefs.status_horizontal;
    let editing = mode == OverlayMode::Interactive;

    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("warframe-status-panel"))
        .current_pos(current_position)
        .movable(interactive)
        .interactable(input_enabled)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            if !input_enabled {
                ui.disable();
            }
            apply_scale(ui, scale);
            let frame = panel_frame(transparent_background).show(ui, |ui| {
                if horizontal {
                    // Content-width bar; while editing keep a usable min width.
                    if editing {
                        ui.set_min_width(panel_content_width(
                            HORIZONTAL_EDIT_MIN_WIDTH,
                            transparent_background,
                        ));
                    }
                } else {
                    // Fixed width (user-resizable); height follows content.
                    let content_width = panel_content_width(panel_size.x, transparent_background);
                    ui.set_min_width(content_width);
                    ui.set_max_width(content_width);
                }

                if editing {
                    paint_status_header(ui, prefs, horizontal, &mut actions);
                    ui.add_space(4.0);
                } else if !horizontal {
                    widget_identity(ui, WidgetGlyph::Warframe, "WARFRAME", Some("WORLDSTATE"));
                    ui.add_space(4.0);
                }

                if let Some(error) = &snapshot.error {
                    ui.colored_label(
                        accent_error(),
                        egui::RichText::new(error).size(scaled_content_font_size(ui, BODY_SIZE)),
                    );
                }
                if snapshot.cycles.is_empty()
                    && snapshot.baro.is_none()
                    && snapshot.daily_reset_at_secs.is_none()
                {
                    ui.label(meta_text("Worldstate unavailable"));
                } else if items.is_empty() {
                    ui.label(meta_text("No timers selected"));
                } else if horizontal {
                    paint_status_bar(ui, &items);
                } else {
                    for item in &items {
                        paint_status_row(ui, &item.name, &item.state, item.state_color, &item.time);
                    }
                }
            });
            // Vertical only: width grip. Horizontal size is content-driven.
            resize = resize_grip(
                ui,
                frame.response.rect,
                input_enabled && editing && !horizontal,
            );
        });

    let measured = response.response.rect.size().max(vec2(1.0, 1.0));
    WarframeStatusResponse {
        size: measured,
        position: response.response.rect.min,
        dragged: response.response.dragged() && !resize.dragging,
        drag_stopped: response.response.drag_stopped() && !resize.dragging && !resize.drag_stopped,
        resize,
        actions,
    }
}

fn collect_status_items(
    snapshot: &WorldstateSnapshot,
    prefs: &WarframePrefs,
    now_secs: u64,
) -> Vec<StatusItem> {
    let mut items = Vec::new();
    for cycle in &snapshot.cycles {
        let Some(row) = StatusRow::from_cycle_id(&cycle.id) else {
            continue;
        };
        if !prefs.status_row_visible(row) {
            continue;
        }
        let state = cycle_state_label(cycle.state.as_deref());
        let state_color = cycle_state_color(state.to_ascii_lowercase().as_str());
        items.push(StatusItem {
            name: cycle.label.clone(),
            state,
            state_color,
            time: format_remaining(now_secs, cycle.expires_at_secs, prefs.show_status_seconds),
        });
    }

    if prefs.status_row_visible(StatusRow::DailyReset)
        && let Some(reset) = snapshot.daily_reset_at_secs
    {
        items.push(StatusItem {
            name: "Reset".to_owned(),
            state: "00:00 UTC".to_owned(),
            state_color: Color32::from_gray(200),
            time: format_remaining(now_secs, reset, prefs.show_status_seconds),
        });
    }

    if prefs.status_row_visible(StatusRow::Baro)
        && let Some(baro) = &snapshot.baro
    {
        let location = baro.location.as_deref().unwrap_or("—");
        if baro.present {
            items.push(StatusItem {
                name: "Baro".to_owned(),
                state: format!("Present · {location}"),
                state_color: accent_ok(),
                time: format_remaining(now_secs, baro.expiry_secs, prefs.show_status_seconds),
            });
        } else {
            items.push(StatusItem {
                name: "Baro".to_owned(),
                state: format!("Arrives · {location}"),
                state_color: accent_warn(),
                time: format_remaining(now_secs, baro.activation_secs, prefs.show_status_seconds),
            });
        }
    }

    items
}

/// Title + **filters** (which rows) on the left; **options** gear on the right.
fn paint_status_header(
    ui: &mut egui::Ui,
    prefs: &WarframePrefs,
    horizontal: bool,
    actions: &mut Vec<StatusPrefsAction>,
) {
    ui.horizontal(|ui| {
        widget_identity(
            ui,
            WidgetGlyph::Warframe,
            "WARFRAME",
            Some(if horizontal {
                "STATUS BAR"
            } else {
                "WORLDSTATE"
            }),
        );
    });
    // Filters: which cycles / utilities to show.
    ui.horizontal_wrapped(|ui| {
        for row in StatusRow::ALL {
            let mut visible = prefs.status_row_visible(row);
            if filter_chip(
                ui,
                &mut visible,
                egui::RichText::new(row.label()).size(scaled_content_font_size(ui, META_SIZE)),
                ACCENT,
            )
            .clicked()
            {
                actions.push(StatusPrefsAction::ToggleRow(row));
            }
        }
    });
}

pub(crate) fn paint_status_options(
    ui: &mut egui::Ui,
    prefs: &WarframePrefs,
    actions: &mut Vec<StatusPrefsAction>,
) {
    let mut horizontal = prefs.status_horizontal;
    if ui.checkbox(&mut horizontal, "Horizontal bar").changed() {
        actions.push(StatusPrefsAction::ToggleHorizontal);
    }
    let mut seconds = prefs.show_status_seconds;
    if ui.checkbox(&mut seconds, "Seconds").changed() {
        actions.push(StatusPrefsAction::ToggleSeconds);
    }
}

/// Single forced row: cells separated like the overlay control bar.
///
/// Each cell is:
/// ```text
/// Cetus Day
/// 12m 03s
/// ```
/// so the bar stays short and wide only as needed by the checked rows.
fn paint_status_bar(ui: &mut egui::Ui, items: &[StatusItem]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                paint_bar_separator(ui);
            }
            paint_status_bar_cell(ui, item);
        }
    });
}

fn paint_bar_separator(ui: &mut egui::Ui) {
    let scale = current_content_scale(ui);
    let height = (BAR_NAME_SIZE + BAR_CELL_GAP + BAR_TIMER_SIZE + 2.0) * scale;
    let (rect, _) = ui.allocate_exact_size(vec2(BAR_SEP_PAD * 2.0 * scale, height), Sense::hover());
    let x = rect.center().x;
    ui.painter().line_segment(
        [
            egui::pos2(x, rect.top() + 1.0),
            egui::pos2(x, rect.bottom() - 1.0),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(36)),
    );
}

fn paint_status_bar_cell(ui: &mut egui::Ui, item: &StatusItem) {
    let scale = current_content_scale(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = BAR_CELL_GAP * scale;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0 * scale;
            ui.label(
                egui::RichText::new(&item.name)
                    .size(scaled_content_font_size(ui, BAR_NAME_SIZE))
                    .strong()
                    .color(TEXT_PRIMARY),
            );
            ui.label(
                egui::RichText::new(&item.state)
                    .size(scaled_content_font_size(ui, BAR_STATE_SIZE))
                    .strong()
                    .color(item.state_color),
            );
        });
        ui.label(
            egui::RichText::new(&item.time)
                .monospace()
                .size(scaled_content_font_size(ui, BAR_TIMER_SIZE))
                .strong()
                .color(timer_color()),
        );
    });
}

fn paint_status_row(ui: &mut egui::Ui, name: &str, state: &str, state_color: Color32, time: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(name)
                .size(scaled_content_font_size(ui, BODY_SIZE))
                .strong()
                .color(TEXT_PRIMARY),
        );
        ui.label(
            egui::RichText::new(state)
                .size(scaled_content_font_size(ui, BODY_SIZE))
                .strong()
                .color(state_color),
        );
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(time)
                    .monospace()
                    .size(scaled_content_font_size(ui, TIMER_SIZE))
                    .strong()
                    .color(timer_color()),
            );
        });
    });
    ui.add_space(3.0);
}

fn cycle_state_label(state: Option<&str>) -> String {
    match state {
        Some("day") | Some("jour") => "Day".to_owned(),
        Some("night") | Some("nuit") => "Night".to_owned(),
        Some("chaud") | Some("warm") => "Warm".to_owned(),
        Some("froid") | Some("cold") => "Cold".to_owned(),
        // Proper names / game terms stay as-is (English game content).
        Some("fass") => "Fass".to_owned(),
        Some("vome") => "Vome".to_owned(),
        Some("corpus") => "Corpus".to_owned(),
        Some("grineer") => "Grineer".to_owned(),
        Some(other) => {
            let mut s = other.to_owned();
            if let Some(first) = s.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            s
        }
        None => "—".to_owned(),
    }
}

pub fn apply_status_prefs_action(prefs: &mut WarframePrefs, action: StatusPrefsAction) {
    match action {
        StatusPrefsAction::ToggleHorizontal => {
            prefs.status_horizontal = !prefs.status_horizontal;
        }
        StatusPrefsAction::ToggleSeconds => {
            prefs.show_status_seconds = !prefs.show_status_seconds;
        }
        StatusPrefsAction::ToggleRow(row) => {
            let next = !prefs.status_row_visible(row);
            prefs.set_status_row_visible(row, next);
        }
    }
}
