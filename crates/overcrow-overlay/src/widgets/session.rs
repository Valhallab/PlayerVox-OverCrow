use std::time::{Duration, Instant};

use eframe::egui;
use overcrow_config::{WidgetId, WidgetProfile};
use overcrow_protocol::{CoreSnapshot, OverlayMode};

use crate::session_clock::SessionClock;

use super::widget_visible;

pub struct SessionResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
}

pub fn session_visible(snapshot: &CoreSnapshot, profile: &WidgetProfile) -> bool {
    widget_visible(
        WidgetId::Session,
        snapshot.overlay_mode,
        snapshot.active_game.is_some(),
        profile,
    )
}

pub fn session_repaint_after(
    snapshot: &CoreSnapshot,
    profile: &WidgetProfile,
    clock: &SessionClock,
    now: Instant,
) -> Option<Duration> {
    if session_visible(snapshot, profile) {
        clock.repaint_after(now)
    } else {
        None
    }
}

pub fn session_draggable(snapshot: &CoreSnapshot) -> bool {
    snapshot.active_game.is_some() && snapshot.overlay_mode == OverlayMode::Interactive
}

pub fn paint_session(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    elapsed: Option<Duration>,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> SessionResponse {
    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("stopwatch-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(draggable)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            super::chrome::compact_panel_frame(transparent_background).show(ui, |ui| {
                ui.label(
                    egui::RichText::new("SESSION")
                        .size(11.0)
                        .color(egui::Color32::from_gray(170)),
                );
                ui.label(
                    egui::RichText::new(format_session_elapsed(elapsed))
                        .monospace()
                        .strong()
                        .size(30.0),
                );
            });
        });

    SessionResponse {
        size: response.response.rect.size(),
        position: response.response.rect.min,
        dragged: response.response.dragged(),
        drag_stopped: response.response.drag_stopped(),
    }
}

pub fn format_session_elapsed(elapsed: Option<Duration>) -> String {
    let Some(elapsed) = elapsed else {
        return "--:--:--".to_owned();
    };
    let seconds = elapsed.as_secs();
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}
