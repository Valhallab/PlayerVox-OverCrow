use eframe::egui;
use overcrow_protocol::OverlayMode;

use crate::media::{MediaAction, MediaPlaybackStatus, MediaSnapshot};

use super::chrome::{ControlIcon, TEXT_MUTED, TEXT_PRIMARY, control_icon, icon_button};

const MEDIA_WIDTH_MIN: f32 = 240.0;
const MEDIA_WIDTH_MAX: f32 = 560.0;
const MEDIA_FRAME_BUDGET: f32 = 38.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaControl {
    pub icon: ControlIcon,
    pub accessible_label: &'static str,
    pub action: MediaAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaPresentation {
    pub title: String,
    pub artist: Option<String>,
    pub state_icon: Option<ControlIcon>,
    pub empty_message: Option<&'static str>,
    pub provider_message: Option<String>,
    pub controls: Vec<MediaControl>,
}

impl MediaPresentation {
    pub fn new(snapshot: &MediaSnapshot, mode: OverlayMode) -> Self {
        let empty_message = snapshot.bus_name.is_none().then_some("No active media");
        let mut controls = Vec::with_capacity(3);
        if mode == OverlayMode::Interactive && snapshot.bus_name.is_some() {
            if snapshot.capabilities.can_go_previous {
                controls.push(MediaControl {
                    icon: ControlIcon::Previous,
                    accessible_label: "Previous",
                    action: MediaAction::Previous,
                });
            }
            if MediaAction::PlayPause.command_for(snapshot).is_some() {
                controls.push(MediaControl {
                    icon: if snapshot.playback_status == MediaPlaybackStatus::Playing {
                        ControlIcon::Pause
                    } else {
                        ControlIcon::Play
                    },
                    accessible_label: if snapshot.playback_status == MediaPlaybackStatus::Playing {
                        "Pause"
                    } else {
                        "Play"
                    },
                    action: MediaAction::PlayPause,
                });
            }
            if snapshot.capabilities.can_go_next {
                controls.push(MediaControl {
                    icon: ControlIcon::Next,
                    accessible_label: "Next",
                    action: MediaAction::Next,
                });
            }
        }

        Self {
            title: snapshot
                .title
                .clone()
                .unwrap_or_else(|| "Unknown title".to_owned()),
            artist: snapshot.artist.clone(),
            state_icon: if snapshot.bus_name.is_some() {
                match snapshot.playback_status {
                    MediaPlaybackStatus::Playing => Some(ControlIcon::Play),
                    MediaPlaybackStatus::Paused => Some(ControlIcon::Pause),
                    MediaPlaybackStatus::Stopped => None,
                }
            } else {
                None
            },
            empty_message,
            provider_message: snapshot.error.clone(),
            controls,
        }
    }
}

pub struct MediaResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub action: Option<MediaAction>,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_media(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    snapshot: &MediaSnapshot,
    mode: OverlayMode,
    scale: f32,
    transparent_background: bool,
    draggable: bool,
    input_enabled: bool,
    margin: f32,
) -> MediaResponse {
    let presentation = MediaPresentation::new(snapshot, mode);
    let mut action = None;
    let viewport = ui.max_rect();
    let safe_width = (viewport.width() - margin * 2.0).max(1.0);
    let response = egui::Area::new(egui::Id::new("media-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(input_enabled && (draggable || !presentation.controls.is_empty()))
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            if !input_enabled {
                ui.disable();
            }
            let panel_width = media_panel_width(ui, &presentation, scale, safe_width);
            super::chrome::apply_scale(ui, scale);
            super::chrome::compact_panel_frame(transparent_background).show(ui, |ui| {
                ui.set_width((panel_width - MEDIA_FRAME_BUDGET).max(1.0));
                if let Some(message) = presentation.empty_message {
                    ui.label(egui::RichText::new(message).color(TEXT_MUTED));
                } else {
                    ui.label(
                        egui::RichText::new(&presentation.title)
                            .strong()
                            .size(20.0 * scale)
                            .color(TEXT_PRIMARY),
                    );
                    ui.horizontal(|ui| {
                        if let Some(artist) = &presentation.artist {
                            ui.label(egui::RichText::new(artist).color(TEXT_MUTED));
                        }
                        if let Some(icon) = presentation.state_icon {
                            if presentation.artist.is_some() {
                                ui.separator();
                            }
                            let label = match icon {
                                ControlIcon::Play => "Playing",
                                ControlIcon::Pause => "Paused",
                                _ => "Playback state",
                            };
                            control_icon(ui, icon, label);
                        }
                    });
                }

                if let Some(message) = &presentation.provider_message {
                    ui.label(egui::RichText::new(message).small().color(TEXT_MUTED));
                }

                if !presentation.controls.is_empty() {
                    ui.horizontal(|ui| {
                        for control in &presentation.controls {
                            if icon_button(ui, control.icon, control.accessible_label).clicked() {
                                action = Some(control.action);
                            }
                        }
                    });
                }
            });
        });

    MediaResponse {
        size: response.response.rect.size(),
        position: response.response.rect.min,
        dragged: response.response.dragged(),
        drag_stopped: response.response.drag_stopped(),
        action,
    }
}

fn media_panel_width(
    ui: &egui::Ui,
    presentation: &MediaPresentation,
    scale: f32,
    safe_width: f32,
) -> f32 {
    let scale = scale.clamp(0.75, 1.75);
    let mut title_font = egui::TextStyle::Heading.resolve(ui.style());
    title_font.size = 20.0 * scale;
    let mut body_font = egui::TextStyle::Body.resolve(ui.style());
    body_font.size *= scale;
    let text_width = |text: &str, font: egui::FontId| {
        ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(text.to_owned(), font, TEXT_PRIMARY)
                .size()
                .x
        })
    };

    let mut content_width = text_width(&presentation.title, title_font);
    if let Some(artist) = &presentation.artist {
        content_width = content_width.max(text_width(artist, body_font.clone()) + 28.0 * scale);
    }
    if let Some(message) = presentation.empty_message {
        content_width = content_width.max(text_width(message, body_font.clone()));
    }
    if let Some(message) = &presentation.provider_message {
        content_width = content_width.max(text_width(message, body_font));
    }
    if !presentation.controls.is_empty() {
        let controls = presentation.controls.len() as f32;
        content_width = content_width.max(
            controls * super::chrome::CONTROL_HEIGHT + (controls - 1.0).max(0.0) * 8.0 * scale,
        );
    }

    let maximum = MEDIA_WIDTH_MAX.min(safe_width).max(1.0);
    let minimum = MEDIA_WIDTH_MIN.min(maximum);
    (content_width + MEDIA_FRAME_BUDGET).clamp(minimum, maximum)
}
