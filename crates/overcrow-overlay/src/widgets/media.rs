use eframe::egui;
use overcrow_protocol::OverlayMode;

use crate::media::{MediaAction, MediaPlaybackStatus, MediaSnapshot};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaControl {
    pub label: &'static str,
    pub action: MediaAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaPresentation {
    pub title: String,
    pub artist: Option<String>,
    pub status: &'static str,
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
                    label: "Previous",
                    action: MediaAction::Previous,
                });
            }
            if MediaAction::PlayPause.command_for(snapshot).is_some() {
                controls.push(MediaControl {
                    label: if snapshot.playback_status == MediaPlaybackStatus::Playing {
                        "Pause"
                    } else {
                        "Play"
                    },
                    action: MediaAction::PlayPause,
                });
            }
            if snapshot.capabilities.can_go_next {
                controls.push(MediaControl {
                    label: "Next",
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
            status: match snapshot.playback_status {
                MediaPlaybackStatus::Playing => "PLAYING",
                MediaPlaybackStatus::Paused => "PAUSED",
                MediaPlaybackStatus::Stopped => "STOPPED",
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

pub fn paint_media(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    snapshot: &MediaSnapshot,
    mode: OverlayMode,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> MediaResponse {
    let presentation = MediaPresentation::new(snapshot, mode);
    let mut action = None;
    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("media-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(draggable || !presentation.controls.is_empty())
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            super::chrome::compact_panel_frame(transparent_background).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("MÉDIAS")
                            .size(11.0)
                            .color(egui::Color32::from_gray(170)),
                    );
                    if presentation.empty_message.is_none() {
                        ui.label(
                            egui::RichText::new(presentation.status)
                                .size(11.0)
                                .color(egui::Color32::from_gray(190)),
                        );
                    }
                });

                if let Some(message) = presentation.empty_message {
                    ui.label(message);
                } else {
                    ui.label(egui::RichText::new(&presentation.title).strong().size(20.0));
                    if let Some(artist) = &presentation.artist {
                        ui.label(egui::RichText::new(artist).color(egui::Color32::from_gray(190)));
                    }
                }

                if let Some(message) = &presentation.provider_message {
                    ui.label(
                        egui::RichText::new(message)
                            .small()
                            .color(egui::Color32::from_gray(140)),
                    );
                }

                if !presentation.controls.is_empty() {
                    ui.horizontal(|ui| {
                        for control in &presentation.controls {
                            if ui.button(control.label).clicked() {
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
