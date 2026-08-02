use std::sync::Arc;

use eframe::egui::{self, Color32, Stroke, Vec2, vec2};
use overcrow_config::DiscordVoiceAlignment;
use overcrow_protocol::OverlayMode;

use crate::discord::{
    avatars::{AvatarKey, DiscordAvatars},
    client::{DiscordCommand, DiscordConnectionState, DiscordSnapshot},
    model::VoiceParticipant,
};
use crate::icons::{AppIcon, paint_icon_at};

use super::chrome::{
    ACCENT, PANEL_STROKE, TEXT_MUTED, TEXT_PRIMARY, accent_warn, apply_scale,
    current_content_scale, fixed_panel_constraints, meta_text, panel_content_height,
    panel_content_width, panel_frame, primary_button,
};

pub type DiscordVoiceAction = DiscordCommand;

const DISCORD_AVATAR_BASE_SIZE: f32 = 35.0;
const DISCORD_NAME_MAX_BASE_WIDTH: f32 = 180.0;
const DISCORD_CONTENT_MIN_BASE_WIDTH: f32 = 96.0;
const VOICE_STATE_ICON_BASE_SIZE: f32 = 14.0;

pub struct DiscordWidgetState {
    snapshot: Arc<DiscordSnapshot>,
    revision: u64,
    avatars: Option<DiscordAvatars>,
}

impl Default for DiscordWidgetState {
    fn default() -> Self {
        Self {
            snapshot: Arc::new(DiscordSnapshot::default()),
            revision: 0,
            avatars: None,
        }
    }
}

impl DiscordWidgetState {
    pub fn with_avatars(avatars: DiscordAvatars) -> Self {
        Self {
            avatars: Some(avatars),
            ..Self::default()
        }
    }

    pub fn snapshot(&self) -> &Arc<DiscordSnapshot> {
        &self.snapshot
    }

    pub fn apply_snapshot(&mut self, revision: u64, snapshot: Arc<DiscordSnapshot>) {
        if revision <= self.revision {
            return;
        }
        if let Some(avatars) = &mut self.avatars {
            let referenced = snapshot
                .channel
                .iter()
                .flat_map(|channel| channel.participants.iter())
                .filter_map(|participant| {
                    AvatarKey::new(&participant.id, participant.avatar_hash.as_deref()?).ok()
                })
                .collect::<Vec<_>>();
            avatars.retain_referenced(referenced.iter());
        }
        self.revision = revision;
        self.snapshot = snapshot;
    }

    pub fn poll_avatars(&mut self, context: &egui::Context, now: std::time::Instant) {
        if let Some(avatars) = &mut self.avatars {
            avatars.poll(context, now);
        }
    }

    pub fn set_avatars_enabled(&mut self, enabled: bool) {
        if let Some(avatars) = &mut self.avatars {
            avatars.set_enabled(enabled);
        }
    }

    pub fn visible_in_passive(&self, participant_limit: u8) -> bool {
        DiscordVoicePresentation::new(&self.snapshot, participant_limit).visible_in_passive()
    }

    fn avatar_texture(
        &mut self,
        participant: &VoiceParticipant,
        now: std::time::Instant,
    ) -> Option<egui::TextureHandle> {
        let key = AvatarKey::new(&participant.id, participant.avatar_hash.as_deref()?).ok()?;
        self.avatars.as_mut()?.texture(&key, now)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscordVoicePresentation {
    pub message: Option<&'static str>,
    pub has_channel: bool,
    pub participants: Vec<VoiceParticipant>,
    pub overflow: usize,
    pub can_connect: bool,
}

impl DiscordVoicePresentation {
    pub fn new(snapshot: &DiscordSnapshot, participant_limit: u8) -> Self {
        let message = if snapshot.channel.is_some()
            && snapshot.connection != DiscordConnectionState::Ready
        {
            Some("Resynchronizing…")
        } else {
            match snapshot.connection {
                DiscordConnectionState::Inert => {
                    Some("Discord voice starts with an authorized game.")
                }
                DiscordConnectionState::ClientNotConfigured => {
                    Some("Discord support is not configured in this build.")
                }
                DiscordConnectionState::Connecting => Some("Looking for the Discord desktop app…"),
                DiscordConnectionState::AuthorizationRequired => {
                    Some("Connect Discord to show your current voice channel.")
                }
                DiscordConnectionState::Authorizing => {
                    Some("Approve PlayerVox OverCrow in Discord.")
                }
                DiscordConnectionState::Authenticating => Some("Signing in to Discord…"),
                DiscordConnectionState::DiscordUnavailable => {
                    Some("Open the Discord desktop app to use voice overlay.")
                }
                DiscordConnectionState::Failed => {
                    Some("Discord credentials need attention. Try signing out again.")
                }
                DiscordConnectionState::Ready if snapshot.channel.is_none() => {
                    Some("Join a Discord voice channel.")
                }
                DiscordConnectionState::Ready => None,
            }
        };
        let limit = usize::from(participant_limit.clamp(2, 16));
        let participants = snapshot
            .channel
            .as_ref()
            .map(|channel| channel.participants.iter().take(limit).cloned().collect())
            .unwrap_or_default();
        let overflow = snapshot.channel.as_ref().map_or(0, |channel| {
            channel.participants.len().saturating_sub(limit)
        });
        Self {
            message,
            has_channel: snapshot.channel.is_some(),
            participants,
            overflow,
            can_connect: snapshot.connection == DiscordConnectionState::AuthorizationRequired
                && snapshot.client_configured,
        }
    }

    pub fn visible_in_passive(&self) -> bool {
        self.has_channel
    }
}

pub struct DiscordVoiceResponse {
    pub size: Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub actions: Vec<DiscordVoiceAction>,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_discord_voice(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    panel_size: Vec2,
    state: &mut DiscordWidgetState,
    participant_limit: u8,
    alignment: DiscordVoiceAlignment,
    scale: f32,
    mode: OverlayMode,
    transparent_background: bool,
    draggable: bool,
    input_enabled: bool,
    margin: f32,
) -> DiscordVoiceResponse {
    let presentation = DiscordVoicePresentation::new(&state.snapshot, participant_limit);
    let panel_size = super::chrome::clamp_panel_size(panel_size);
    let interactive = mode == OverlayMode::Interactive;
    let viewport = ui.max_rect();
    let safe_height = (viewport.height() - margin * 2.0 - 28.0).max(1.0);
    let mut actions = Vec::new();
    let now = std::time::Instant::now();

    let response = egui::Area::new(egui::Id::new("discord-voice-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(input_enabled && interactive)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            if !input_enabled {
                ui.disable();
            }
            apply_scale(ui, scale);
            panel_frame(transparent_background).show(ui, |ui| {
                if presentation.has_channel {
                    let safe_outer_width = (viewport.width() - margin * 2.0).max(1.0);
                    let safe_content_width =
                        panel_content_width(safe_outer_width, transparent_background);
                    let content_width =
                        connected_content_width(ui, &presentation, scale, safe_content_width);
                    ui.set_min_width(content_width);
                    ui.set_max_width(content_width);
                    ui.set_max_height(panel_content_height(safe_height, transparent_background));
                } else {
                    fixed_panel_constraints(
                        ui,
                        panel_size,
                        mode,
                        safe_height,
                        transparent_background,
                    );
                }
                if let Some(message) = presentation.message {
                    if presentation.has_channel && alignment == DiscordVoiceAlignment::Right {
                        ui.with_layout(participant_row_layout(alignment), |ui| {
                            ui.label(meta_text(message));
                        });
                    } else {
                        ui.label(meta_text(message));
                    }
                }
                if presentation.can_connect && interactive {
                    ui.add_space(7.0);
                    if ui.add(primary_button("Connect Discord")).clicked() {
                        actions.push(DiscordCommand::Connect);
                    }
                }
                if presentation.has_channel {
                    paint_participants(ui, state, &presentation, alignment, now);
                }
            });
        });

    DiscordVoiceResponse {
        size: response.response.rect.size().max(vec2(1.0, 1.0)),
        position: response.response.rect.min,
        dragged: response.response.dragged(),
        drag_stopped: response.response.drag_stopped(),
        actions,
    }
}

fn connected_content_width(
    ui: &egui::Ui,
    presentation: &DiscordVoicePresentation,
    scale: f32,
    safe_width: f32,
) -> f32 {
    let participant_width = presentation
        .participants
        .iter()
        .map(|participant| participant_content_width(ui, participant, scale))
        .fold(0.0_f32, f32::max);
    let message_width = presentation
        .message
        .map(|message| text_width(ui, message, egui::TextStyle::Small))
        .unwrap_or_default();
    let overflow_width = if presentation.overflow > 0 {
        text_width(
            ui,
            &format!("+{} more", presentation.overflow),
            egui::TextStyle::Small,
        )
    } else {
        0.0
    };
    let scrollbar_width = discord_voice_scroll_style(ui.spacing().scroll).allocated_width();
    let desired = participant_width
        .max(message_width)
        .max(overflow_width)
        .max(DISCORD_CONTENT_MIN_BASE_WIDTH * scale.clamp(0.75, 1.75))
        + scrollbar_width;

    desired.min(safe_width).max(1.0)
}

fn participant_content_width(ui: &egui::Ui, participant: &VoiceParticipant, scale: f32) -> f32 {
    let icon_count = usize::from(participant.muted) + usize::from(participant.deafened);
    let item_count = 2 + icon_count;
    discord_avatar_size(scale)
        + participant_name_width(ui, &participant.display_name, scale)
        + icon_count as f32 * VOICE_STATE_ICON_BASE_SIZE * scale
        + item_count.saturating_sub(1) as f32 * 6.0 * scale
}

fn participant_name_width(ui: &egui::Ui, display_name: &str, scale: f32) -> f32 {
    text_width(ui, display_name, egui::TextStyle::Body)
        .min(DISCORD_NAME_MAX_BASE_WIDTH * scale.clamp(0.75, 1.75))
}

fn text_width(ui: &egui::Ui, text: &str, style: egui::TextStyle) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), style.resolve(ui.style()), TEXT_PRIMARY)
        .size()
        .x
}

fn paint_participants(
    ui: &mut egui::Ui,
    state: &mut DiscordWidgetState,
    presentation: &DiscordVoicePresentation,
    alignment: DiscordVoiceAlignment,
    now: std::time::Instant,
) {
    ui.scope(|ui| {
        ui.spacing_mut().scroll = discord_voice_scroll_style(ui.spacing().scroll);
        egui::ScrollArea::vertical()
            .auto_shrink([true, true])
            .show(ui, |ui| {
                let scale = current_content_scale(ui);
                ui.spacing_mut().item_spacing.y = 3.0 * scale;
                for participant in &presentation.participants {
                    paint_participant_row(ui, state, participant, alignment, scale, now);
                }
                if presentation.overflow > 0 {
                    if alignment == DiscordVoiceAlignment::Right {
                        ui.with_layout(participant_row_layout(alignment), |ui| {
                            ui.label(meta_text(format!("+{} more", presentation.overflow)));
                        });
                    } else {
                        ui.label(meta_text(format!("+{} more", presentation.overflow)));
                    }
                }
            });
    });
}

pub(super) fn discord_voice_scroll_style(
    mut style: egui::style::ScrollStyle,
) -> egui::style::ScrollStyle {
    style.floating = false;
    style
}

fn paint_participant_row(
    ui: &mut egui::Ui,
    state: &mut DiscordWidgetState,
    participant: &VoiceParticipant,
    alignment: DiscordVoiceAlignment,
    scale: f32,
    now: std::time::Instant,
) {
    if alignment == DiscordVoiceAlignment::Right {
        ui.allocate_ui_with_layout(
            participant_row_size(ui.available_width(), scale),
            participant_row_layout(alignment),
            |ui| {
                paint_participant_content(ui, state, participant, true, scale, now);
            },
        );
    } else {
        ui.horizontal(|ui| {
            paint_participant_content(
                ui,
                state,
                participant,
                alignment == DiscordVoiceAlignment::Left,
                scale,
                now,
            );
        });
    }
}

pub(super) fn paint_participant_content(
    ui: &mut egui::Ui,
    state: &mut DiscordWidgetState,
    participant: &VoiceParticipant,
    avatar_first: bool,
    scale: f32,
    now: std::time::Instant,
) {
    ui.spacing_mut().item_spacing.x = 6.0 * scale;
    if avatar_first {
        paint_avatar(ui, state, participant, discord_avatar_size(scale), now);
    } else {
        paint_voice_state_icons(ui, participant, true);
    }
    let name = egui::RichText::new(&participant.display_name).color(if participant.speaking {
        TEXT_PRIMARY
    } else {
        TEXT_MUTED
    });
    ui.add_sized(
        vec2(
            participant_name_width(ui, &participant.display_name, scale),
            discord_avatar_size(scale),
        ),
        egui::Label::new(name).truncate(),
    );
    if avatar_first {
        paint_voice_state_icons(ui, participant, false);
    } else {
        paint_avatar(ui, state, participant, discord_avatar_size(scale), now);
    }
}

fn paint_voice_state_icons(ui: &mut egui::Ui, participant: &VoiceParticipant, reverse: bool) {
    let icons = if reverse {
        [VoiceStateIcon::Deafened, VoiceStateIcon::Muted]
    } else {
        [VoiceStateIcon::Muted, VoiceStateIcon::Deafened]
    };
    for icon in icons {
        let visible = match icon {
            VoiceStateIcon::Muted => participant.muted,
            VoiceStateIcon::Deafened => participant.deafened,
        };
        if visible {
            paint_voice_state_icon(ui, icon);
        }
    }
}

pub(super) fn discord_avatar_size(scale: f32) -> f32 {
    DISCORD_AVATAR_BASE_SIZE * scale.clamp(0.75, 1.75)
}

pub(super) fn participant_row_layout(alignment: DiscordVoiceAlignment) -> egui::Layout {
    match alignment {
        DiscordVoiceAlignment::Left => egui::Layout::left_to_right(egui::Align::Center),
        DiscordVoiceAlignment::Right => egui::Layout::right_to_left(egui::Align::Center),
    }
}

pub(super) fn participant_row_size(available_width: f32, scale: f32) -> Vec2 {
    vec2(available_width.max(1.0), discord_avatar_size(scale))
}

pub(super) fn participant_avatar_stroke(speaking: bool, scale: f32) -> Stroke {
    let scale = scale.clamp(0.75, 1.75);
    if speaking {
        Stroke::new(1.5 * scale, ACCENT)
    } else {
        Stroke::new(1.0 * scale, PANEL_STROKE)
    }
}

pub(super) fn participant_avatar_radius(size: f32, stroke_width: f32) -> f32 {
    if !size.is_finite() || !stroke_width.is_finite() {
        return 0.0;
    }
    ((size - stroke_width.max(0.0)) * 0.5).max(0.0)
}

fn paint_avatar(
    ui: &mut egui::Ui,
    state: &mut DiscordWidgetState,
    participant: &VoiceParticipant,
    size: f32,
    now: std::time::Instant,
) {
    let scale = current_content_scale(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let ring = participant_avatar_stroke(participant.speaking, scale);
    ui.painter()
        .circle_filled(rect.center(), size * 0.5, Color32::from_rgb(38, 38, 43));
    if let Some(texture) = state.avatar_texture(participant, now) {
        let inset = 2.0 * scale;
        ui.put(
            rect.shrink(inset),
            egui::Image::new(&texture)
                .fit_to_exact_size(rect.shrink(inset).size())
                .corner_radius(((size - inset * 2.0) * 0.5) as u8),
        );
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            participant_initials(&participant.display_name),
            egui::FontId::proportional(11.0 * current_content_scale(ui)),
            TEXT_PRIMARY,
        );
    }
    ui.painter().circle_stroke(
        rect.center(),
        participant_avatar_radius(size, ring.width),
        ring,
    );
    let accessible_label = if participant.speaking {
        format!("{}, speaking", participant.display_name)
    } else {
        participant.display_name.clone()
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            ui.is_enabled(),
            accessible_label.clone(),
        )
    });
    response.on_hover_text(&participant.display_name);
}

#[derive(Clone, Copy)]
pub(super) enum VoiceStateIcon {
    Muted,
    Deafened,
}

impl VoiceStateIcon {
    pub(super) const fn app_icon(self) -> AppIcon {
        match self {
            Self::Muted => AppIcon::MicrophoneMuted,
            Self::Deafened => AppIcon::Headphones,
        }
    }
}

fn paint_voice_state_icon(ui: &mut egui::Ui, icon: VoiceStateIcon) {
    let size = VOICE_STATE_ICON_BASE_SIZE * current_content_scale(ui);
    let label = match icon {
        VoiceStateIcon::Muted => "Muted",
        VoiceStateIcon::Deafened => "Deafened",
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), label));
    paint_icon_at(ui.painter(), rect, icon.app_icon(), TEXT_MUTED);
    response.on_hover_text(label);
}

pub(super) fn participant_initials(display_name: &str) -> String {
    let mut words = display_name
        .split_whitespace()
        .filter_map(|word| word.chars().next());
    let Some(first) = words.next() else {
        return "?".to_owned();
    };
    let mut initials = first.to_uppercase().collect::<String>();
    if let Some(last) = words.next_back() {
        initials.extend(last.to_uppercase());
    }
    initials
}

pub(crate) fn paint_discord_options(
    ui: &mut egui::Ui,
    state: &DiscordWidgetState,
    actions: &mut Vec<DiscordVoiceAction>,
) {
    ui.set_min_width(240.0);
    ui.label(egui::RichText::new("DISCORD ACCOUNT").small().strong());
    let snapshot = state.snapshot();
    if snapshot.credentials_available {
        if !snapshot.credentials_persisted {
            ui.colored_label(
                accent_warn(),
                "Secret Service unavailable — reconnect required after restart.",
            );
        }
        if ui.button("Sign out of Discord").clicked() {
            actions.push(DiscordCommand::SignOut);
            ui.close();
        }
    } else {
        ui.label(meta_text("Connect Discord directly from the widget."));
    }
}
