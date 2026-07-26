use std::time::Duration;

use chrono::{DateTime, Local, Timelike};
use eframe::egui;

use super::chrome::{TEXT_MUTED, value_text};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockPresentation {
    pub time: String,
    pub date: Option<String>,
    pub repaint_after: Duration,
}

impl ClockPresentation {
    pub fn new(now: DateTime<Local>, show_date: bool) -> Self {
        let elapsed_in_minute = Duration::from_secs(u64::from(now.second()))
            + Duration::from_nanos(u64::from(now.nanosecond()));

        Self {
            time: now.format("%H:%M").to_string(),
            date: show_date.then(|| now.format("%d/%m/%Y").to_string()),
            repaint_after: Duration::from_secs(60) - elapsed_in_minute,
        }
    }
}

pub struct ClockResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
}

pub fn paint_clock(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    show_date: bool,
    scale: f32,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> ClockResponse {
    let presentation = ClockPresentation::new(Local::now(), show_date);
    ui.ctx().request_repaint_after(presentation.repaint_after);

    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("clock-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(draggable)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            super::chrome::apply_scale(ui, scale);
            super::chrome::compact_panel_frame(transparent_background).show(ui, |ui| {
                ui.label(
                    value_text(presentation.time, 30.0 * scale)
                        .monospace()
                        .color(egui::Color32::WHITE),
                );
                if let Some(date) = presentation.date {
                    ui.label(egui::RichText::new(date).monospace().color(TEXT_MUTED));
                }
            });
        });

    ClockResponse {
        size: response.response.rect.size(),
        position: response.response.rect.min,
        dragged: response.response.dragged(),
        drag_stopped: response.response.drag_stopped(),
    }
}
