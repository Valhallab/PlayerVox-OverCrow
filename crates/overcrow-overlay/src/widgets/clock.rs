use std::time::Duration;

use chrono::{DateTime, Local, Timelike};
use eframe::egui;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockPresentation {
    pub time: String,
    pub date: String,
    pub repaint_after: Duration,
}

impl From<DateTime<Local>> for ClockPresentation {
    fn from(now: DateTime<Local>) -> Self {
        let elapsed_in_minute = Duration::from_secs(u64::from(now.second()))
            + Duration::from_nanos(u64::from(now.nanosecond()));

        Self {
            time: now.format("%H:%M").to_string(),
            date: now.format("%d/%m/%Y").to_string(),
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
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> ClockResponse {
    let presentation = ClockPresentation::from(Local::now());
    ui.ctx().request_repaint_after(presentation.repaint_after);

    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("clock-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(draggable)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            super::chrome::compact_panel_frame(transparent_background).show(ui, |ui| {
                ui.label(
                    egui::RichText::new("LOCAL TIME")
                        .size(11.0)
                        .color(egui::Color32::from_gray(170)),
                );
                ui.label(
                    egui::RichText::new(presentation.time)
                        .monospace()
                        .strong()
                        .size(30.0),
                );
                ui.label(
                    egui::RichText::new(presentation.date)
                        .monospace()
                        .color(egui::Color32::from_gray(190)),
                );
            });
        });

    ClockResponse {
        size: response.response.rect.size(),
        position: response.response.rect.min,
        dragged: response.response.dragged(),
        drag_stopped: response.response.drag_stopped(),
    }
}
