use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, Vec2, vec2};
use overcrow_config::{
    TWITCH_CHANNEL_MAX_CHARS, TWITCH_FAVORITES_MAX, TWITCH_PASSIVE_LIFETIME_MAX_SECS,
    TWITCH_PASSIVE_LIFETIME_MIN_SECS, TwitchPrefs, normalize_twitch_channel,
};
use overcrow_protocol::OverlayMode;

use crate::twitch::model::{
    TWITCH_MESSAGE_MAX_CHARS, TwitchCommand, TwitchConnectionState, TwitchFailureCategory,
    TwitchMessage, TwitchSendReceiptState, TwitchSendState, TwitchSnapshot,
};

use super::{
    WidgetGlyph,
    chrome::{
        ACCENT, ResizeGripOutcome, TEXT_MUTED, TEXT_PRIMARY, accent_error, accent_ok, accent_warn,
        apply_scale, eyebrow_text, fixed_panel_constraints, meta_text, paint_widget_glyph,
        panel_frame, primary_button, resize_grip, singleline_text_edit, standard_button,
        status_pill, tab_button, title_text,
    },
};

const PASSIVE_VISIBLE_MAX: usize = 12;
/// Favorite star (filled). Use a plain glyph; avoid exotic icons that render as tofu.
const FAVORITE_STAR_ON: &str = "★";
const FAVORITE_STAR_YELLOW: Color32 = Color32::from_rgb(255, 200, 60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TwitchChatAction {
    Command(TwitchCommand),
    SetChannel(String),
    ClearChannel,
    ToggleFavorite(String),
    SetPassiveLifetime(u32),
    OpenVerification(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TwitchReplyTarget {
    pub message_id: String,
    pub display_name: String,
}

pub struct TwitchWidgetState {
    snapshot: Arc<TwitchSnapshot>,
    revision: u64,
    generation: u64,
    draft: String,
    channel_draft: String,
    /// Once true, an empty channel field stays empty (user may be clearing it).
    channel_draft_seeded: bool,
    reply_target: Option<TwitchReplyTarget>,
    auto_scroll: bool,
    unread_count: usize,
    previous_message_count: usize,
    previous_tail_key: Option<String>,
    message: Option<String>,
    next_request_id: u64,
    pending_request_id: Option<u64>,
    verification_opening: bool,
}

impl Default for TwitchWidgetState {
    fn default() -> Self {
        Self {
            snapshot: Arc::new(TwitchSnapshot::default()),
            revision: 0,
            generation: 0,
            draft: String::new(),
            channel_draft: String::new(),
            channel_draft_seeded: false,
            reply_target: None,
            auto_scroll: true,
            unread_count: 0,
            previous_message_count: 0,
            previous_tail_key: None,
            message: None,
            next_request_id: 1,
            pending_request_id: None,
            verification_opening: false,
        }
    }
}

impl TwitchWidgetState {
    pub fn snapshot(&self) -> &Arc<TwitchSnapshot> {
        &self.snapshot
    }

    pub fn apply_snapshot(&mut self, revision: u64, snapshot: Arc<TwitchSnapshot>) {
        if revision <= self.revision {
            return;
        }
        let next_tail_key = snapshot
            .messages
            .last()
            .map(message_identity)
            .map(str::to_owned);
        if snapshot.generation != self.generation {
            self.generation = snapshot.generation;
            self.draft.clear();
            self.reply_target = None;
            self.auto_scroll = true;
            self.unread_count = 0;
            self.previous_message_count = snapshot.messages.len();
            self.previous_tail_key = next_tail_key;
            self.pending_request_id = None;
        } else {
            if !self.auto_scroll && next_tail_key != self.previous_tail_key {
                let added = self
                    .previous_tail_key
                    .as_deref()
                    .and_then(|previous| {
                        snapshot
                            .messages
                            .iter()
                            .rposition(|message| message_identity(message) == previous)
                            .map(|index| snapshot.messages.len().saturating_sub(index + 1))
                    })
                    .unwrap_or_else(|| {
                        if snapshot.messages.len() >= self.previous_message_count {
                            snapshot.messages.len()
                        } else {
                            0
                        }
                    });
                self.unread_count = self.unread_count.saturating_add(added);
            }
            self.previous_tail_key = next_tail_key;
        }
        if let Some(receipt) = &snapshot.send_receipt
            && self.pending_request_id == Some(receipt.request_id)
        {
            self.pending_request_id = None;
            match receipt.state {
                TwitchSendReceiptState::Accepted => self.mark_send_accepted(),
                TwitchSendReceiptState::Rejected => {
                    self.set_message(Some("Message was not accepted. Try again.".to_owned()));
                }
            }
        }
        self.previous_message_count = snapshot.messages.len();
        self.revision = revision;
        self.snapshot = snapshot;
    }

    pub fn set_draft(&mut self, value: String) {
        self.draft = value.chars().take(TWITCH_MESSAGE_MAX_CHARS).collect();
    }

    pub fn draft(&self) -> &str {
        &self.draft
    }

    pub fn mark_send_accepted(&mut self) {
        self.draft.clear();
        self.reply_target = None;
        self.auto_scroll = true;
        self.unread_count = 0;
    }

    pub fn mark_send_rejected(&mut self, request_id: u64) {
        if self.pending_request_id == Some(request_id) {
            self.pending_request_id = None;
            self.set_message(Some("Message queue is full. Try again.".to_owned()));
        }
    }

    pub(super) fn begin_send(&mut self, snapshot: &TwitchSnapshot) -> Option<TwitchChatAction> {
        if self.pending_request_id.is_some() {
            return None;
        }
        let channel = snapshot.channel.clone()?;
        let request_id = self.next_request_id;
        self.next_request_id = request_id.checked_add(1).unwrap_or(1);
        self.pending_request_id = Some(request_id);
        Some(TwitchChatAction::Command(TwitchCommand::SendMessage {
            request_id,
            generation: snapshot.generation,
            channel,
            text: self.draft.clone(),
            reply_to: self
                .reply_target
                .as_ref()
                .map(|reply| reply.message_id.clone()),
        }))
    }

    pub fn set_reply(&mut self, message_id: String, display_name: String) {
        self.reply_target = Some(TwitchReplyTarget {
            message_id,
            display_name,
        });
    }

    pub fn reply_target(&self) -> Option<&TwitchReplyTarget> {
        self.reply_target.as_ref()
    }

    pub fn set_auto_scroll(&mut self, auto_scroll: bool) {
        self.auto_scroll = auto_scroll;
        if auto_scroll {
            self.unread_count = 0;
        }
    }

    pub fn auto_scroll(&self) -> bool {
        self.auto_scroll
    }

    pub fn unread_count(&self) -> usize {
        self.unread_count
    }

    pub fn return_to_latest(&mut self) {
        self.set_auto_scroll(true);
    }

    pub fn set_message(&mut self, message: Option<String>) {
        self.message = message.map(|message| message.chars().take(180).collect());
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn sync_channel_draft(&mut self, prefs: &TwitchPrefs) {
        // Seed only once. Re-filling whenever the field is empty made it
        // impossible to clear the box to type a different channel.
        if self.channel_draft_seeded {
            return;
        }
        self.channel_draft_seeded = true;
        if let Some(channel) = &prefs.active_channel {
            self.channel_draft.clone_from(channel);
        }
    }

    pub fn set_channel_draft(&mut self, channel: &str) {
        self.channel_draft.clear();
        self.channel_draft.push_str(channel);
        self.channel_draft_seeded = true;
    }

    pub fn channel_draft(&self) -> &str {
        &self.channel_draft
    }

    pub fn set_verification_opening(&mut self, opening: bool) {
        self.verification_opening = opening;
    }
}

fn message_identity(message: &TwitchMessage) -> &str {
    message
        .client_nonce
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(&message.id)
}

pub struct TwitchChatResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub resize: ResizeGripOutcome,
    pub actions: Vec<TwitchChatAction>,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_twitch_chat(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    panel_size: Vec2,
    state: &mut TwitchWidgetState,
    prefs: &TwitchPrefs,
    scale: f32,
    mode: OverlayMode,
    transparent_background: bool,
    draggable: bool,
    input_enabled: bool,
    margin: f32,
    now: Instant,
) -> TwitchChatResponse {
    state.sync_channel_draft(prefs);
    let panel_size = super::chrome::clamp_panel_size(panel_size);
    let interactive = mode == OverlayMode::Interactive;
    let mut actions = Vec::new();
    let mut resize = ResizeGripOutcome::default();
    let viewport = ui.max_rect();
    let safe_height = (viewport.height() - margin * 2.0 - 28.0).max(1.0);

    let response = egui::Area::new(egui::Id::new("twitch-chat-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(input_enabled && interactive)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            if !input_enabled {
                ui.disable();
            }
            apply_scale(ui, scale);
            let frame = panel_frame(transparent_background).show(ui, |ui| {
                fixed_panel_constraints(ui, panel_size, mode, safe_height, transparent_background);
                paint_header(ui, state, prefs);
                if let Some(message) = &state.message {
                    ui.colored_label(accent_error(), egui::RichText::new(message).small());
                }
                ui.add_space(4.0);
                paint_chat(ui, state, prefs, interactive, panel_size, now, &mut actions);
            });
            resize = resize_grip(ui, frame.response.rect, input_enabled && interactive);
        });

    let measured = response.response.rect.size().max(vec2(1.0, 1.0));
    TwitchChatResponse {
        size: measured,
        position: response.response.rect.min,
        dragged: response.response.dragged() && !resize.dragging,
        drag_stopped: response.response.drag_stopped() && !resize.dragging && !resize.drag_stopped,
        resize,
        actions,
    }
}

pub(super) fn paint_header(ui: &mut egui::Ui, state: &mut TwitchWidgetState, prefs: &TwitchPrefs) {
    let snapshot = Arc::clone(&state.snapshot);
    ui.horizontal(|ui| {
        paint_widget_glyph(ui, WidgetGlyph::Twitch, 28.0, true);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(title_text("TWITCH CHAT"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                paint_favorite_indicator(ui, prefs);
                let channel = prefs
                    .active_channel
                    .as_deref()
                    .or(snapshot.channel.as_deref())
                    .map_or_else(
                        || "Select a channel".to_owned(),
                        |channel| format!("#{channel}"),
                    );
                ui.label(meta_text(channel));
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            status_pill(
                ui,
                connection_label(&snapshot.connection),
                connection_color(&snapshot.connection),
            );
        });
    });
}

pub(super) fn paint_favorite_indicator(ui: &mut egui::Ui, prefs: &TwitchPrefs) {
    let Some(channel) = prefs.active_channel.as_deref() else {
        return;
    };
    if prefs.favorites.iter().any(|item| item == channel) {
        let response = ui.label(
            egui::RichText::new(FAVORITE_STAR_ON)
                .size(super::chrome::scaled_content_font_size(ui, 14.0))
                .color(FAVORITE_STAR_YELLOW),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Favorite channel")
        });
        response.on_hover_text("Favorite channel");
    }
}

pub(crate) fn paint_twitch_options(
    ui: &mut egui::Ui,
    state: &mut TwitchWidgetState,
    prefs: &TwitchPrefs,
    actions: &mut Vec<TwitchChatAction>,
) {
    let snapshot = Arc::clone(&state.snapshot);
    ui.set_min_width(240.0);
    ui.label(egui::RichText::new("TWITCH ACCOUNT").small().strong());
    if let Some(message) = &state.message {
        ui.colored_label(accent_error(), egui::RichText::new(message).small());
    }
    if let Some(login) = &snapshot.authenticated_login {
        ui.label(format!("Signed in as {login}"));
        if !snapshot.credentials_persisted {
            ui.colored_label(
                accent_warn(),
                "Secret Service unavailable — reconnect required after restart.",
            );
        }
        if ui.button("Sign out of Twitch").clicked() {
            actions.push(TwitchChatAction::Command(TwitchCommand::SignOut));
            ui.close();
        }
    } else if snapshot.credentials_available {
        ui.colored_label(
            accent_warn(),
            "Twitch is temporarily unavailable. Retrying automatically.",
        );
        if ui.button("Sign out of Twitch").clicked() {
            actions.push(TwitchChatAction::Command(TwitchCommand::SignOut));
            ui.close();
        }
    } else {
        ui.label(meta_text(
            "Connect or authorize Twitch directly from the widget.",
        ));
    }

    ui.separator();
    ui.label(egui::RichText::new("PASSIVE CHAT").small().strong());
    let mut lifetime = prefs.passive_lifetime_secs;
    let lifetime_response = ui.add(
        egui::Slider::new(
            &mut lifetime,
            TWITCH_PASSIVE_LIFETIME_MIN_SECS..=TWITCH_PASSIVE_LIFETIME_MAX_SECS,
        )
        .suffix("s")
        .text("Passive lifetime"),
    );
    if (lifetime_response.changed() && !lifetime_response.dragged())
        || lifetime_response.drag_stopped()
    {
        actions.push(TwitchChatAction::SetPassiveLifetime(lifetime));
    }
}

fn paint_channel_selector(
    ui: &mut egui::Ui,
    state: &mut TwitchWidgetState,
    prefs: &TwitchPrefs,
    actions: &mut Vec<TwitchChatAction>,
) {
    ui.label(eyebrow_text("CHANNEL"));
    if state.channel_draft.chars().count() > TWITCH_CHANNEL_MAX_CHARS + 1 {
        state.channel_draft = state
            .channel_draft
            .chars()
            .take(TWITCH_CHANNEL_MAX_CHARS + 1)
            .collect();
    }
    ui.horizontal(|ui| {
        let response = ui.add(
            singleline_text_edit(&mut state.channel_draft)
                .desired_width((ui.available_width() - 100.0).max(120.0))
                .hint_text("Channel name"),
        );
        let normalized = normalize_twitch_channel(&state.channel_draft).ok();
        let apply_requested = ui
            .add_enabled(normalized.is_some(), standard_button("Join chat"))
            .clicked()
            || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if apply_requested && let Some(channel) = normalized {
            actions.push(TwitchChatAction::SetChannel(channel));
        }
    });

    if !prefs.favorites.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(meta_text("Favorites"));
            for favorite in &prefs.favorites {
                let selected = prefs.active_channel.as_deref() == Some(favorite.as_str());
                if ui
                    .add(tab_button(format!("#{favorite}"), selected))
                    .clicked()
                {
                    state.set_channel_draft(favorite);
                    actions.push(TwitchChatAction::SetChannel(favorite.clone()));
                }
            }
        });
    }
}

fn paint_current_channel_favorite_control(
    ui: &mut egui::Ui,
    prefs: &TwitchPrefs,
    actions: &mut Vec<TwitchChatAction>,
) {
    let Some(channel) = prefs.active_channel.as_deref() else {
        return;
    };
    let is_favorite = prefs.favorites.iter().any(|item| item == channel);
    let can_toggle = is_favorite || prefs.favorites.len() < TWITCH_FAVORITES_MAX;
    let label = if is_favorite {
        format!("★ #{channel}")
    } else {
        format!("Favorite #{channel}")
    };
    if ui
        .add_enabled(can_toggle, tab_button(label, is_favorite))
        .on_disabled_hover_text("Favorite channel limit reached")
        .on_hover_text(if is_favorite {
            "Remove current channel from favorites"
        } else {
            "Add current channel to favorites"
        })
        .clicked()
    {
        actions.push(TwitchChatAction::ToggleFavorite(channel.to_owned()));
    }
}

fn paint_authentication(
    ui: &mut egui::Ui,
    state: &mut TwitchWidgetState,
    snapshot: &TwitchSnapshot,
    actions: &mut Vec<TwitchChatAction>,
) {
    ui.vertical_centered(|ui| {
        if !snapshot.client_configured {
            ui.label(meta_text(
                "This build needs the PlayerVox Twitch Client ID before chat can connect.",
            ));
        } else if snapshot.credentials_available {
            ui.label(meta_text(
                "Twitch is temporarily unavailable. Retrying automatically.",
            ));
        } else if let Some(authorization) = &snapshot.authorization {
            ui.label("Enter this code on Twitch:");
            ui.monospace(
                egui::RichText::new(&authorization.user_code)
                    .size(18.0)
                    .strong(),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!state.verification_opening, primary_button("Open Twitch"))
                    .clicked()
                {
                    actions.push(TwitchChatAction::OpenVerification(
                        authorization.verification_uri.clone(),
                    ));
                }
                if ui.add(standard_button("Cancel")).clicked() {
                    actions.push(TwitchChatAction::Command(
                        TwitchCommand::CancelAuthentication,
                    ));
                }
            });
        } else if ui.add(primary_button("Connect Twitch")).clicked() {
            actions.push(TwitchChatAction::Command(
                TwitchCommand::BeginAuthentication,
            ));
        }
    });
}

fn paint_chat(
    ui: &mut egui::Ui,
    state: &mut TwitchWidgetState,
    prefs: &TwitchPrefs,
    interactive: bool,
    panel_size: Vec2,
    now: Instant,
    actions: &mut Vec<TwitchChatAction>,
) {
    let snapshot = Arc::clone(&state.snapshot);
    if !snapshot.client_configured {
        if interactive {
            paint_authentication(ui, state, &snapshot, actions);
        } else {
            ui.label(meta_text("Twitch is not configured in this build."));
        }
        return;
    }
    if snapshot.authenticated_login.is_none() {
        if interactive {
            paint_authentication(ui, state, &snapshot, actions);
        } else {
            ui.label(meta_text("Connect Twitch in interactive mode."));
        }
        return;
    }

    if interactive && prefs.active_channel.is_none() {
        paint_channel_selector(ui, state, prefs, actions);
    }
    if prefs.active_channel.is_none() {
        if !interactive {
            ui.label(meta_text("Choose a Twitch channel in interactive mode."));
        }
        return;
    }

    let composer_height = if interactive { 100.0 } else { 0.0 };
    let max_height = (ui.available_height() - composer_height).max(40.0);
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("twitch-chat-history", interactive))
        .max_height(max_height)
        .auto_shrink([false, !interactive]);
    if interactive {
        scroll = scroll.stick_to_bottom(state.auto_scroll);
    } else {
        scroll = scroll
            .stick_to_bottom(true)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
    }
    let output = scroll.show(ui, |ui| {
        ui.set_min_width((panel_size.x - 36.0).max(1.0));
        if interactive {
            for message in &snapshot.messages {
                paint_message(ui, state, message, 1.0, true);
            }
        } else {
            let lifetime = Duration::from_secs(prefs.passive_lifetime_secs.into());
            let visible: Vec<_> = snapshot
                .messages
                .iter()
                .filter_map(|message| {
                    let age = now
                        .checked_duration_since(message.received_at)
                        .unwrap_or_default();
                    passive_message_alpha(age, lifetime).map(|alpha| (message, alpha))
                })
                .rev()
                .take(PASSIVE_VISIBLE_MAX)
                .collect();
            for (message, alpha) in visible.into_iter().rev() {
                paint_message(ui, state, message, alpha, false);
            }
        }
    });

    if interactive {
        let maximum_offset = (output.content_size.y - output.inner_rect.height()).max(0.0);
        let at_bottom = maximum_offset - output.state.offset.y <= 4.0;
        state.set_auto_scroll(at_bottom);
        if state.unread_count > 0
            && ui
                .button(format!("{} new messages", state.unread_count))
                .clicked()
        {
            state.return_to_latest();
        }
        paint_composer(ui, state, prefs, &snapshot, actions);
    }
}

fn paint_message(
    ui: &mut egui::Ui,
    state: &mut TwitchWidgetState,
    message: &TwitchMessage,
    alpha: f32,
    interactive: bool,
) {
    if let Some(reply) = &message.reply {
        // ASCII-only prefix: specialty arrows often render as empty boxes
        // with the overlay font.
        ui.label(
            egui::RichText::new(format!("re {}: {}", reply.display_name, reply.body))
                .small()
                .color(TEXT_MUTED.gamma_multiply(alpha)),
        );
    }
    // Single label avoids horizontal_wrapped empty chips before the name.
    let body_color = TEXT_PRIMARY.gamma_multiply(alpha);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (marker, _) = ui.allocate_exact_size(Vec2::new(2.0, 14.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(marker, 1.0, ACCENT.gamma_multiply(alpha * 0.55));
        ui.label(username_text(message, alpha));
        ui.label(egui::RichText::new(&message.text).color(body_color));
        match message.send_state {
            TwitchSendState::Pending => {
                ui.label(egui::RichText::new("sending…").small().color(accent_warn()));
            }
            TwitchSendState::Failed => {
                ui.label(
                    egui::RichText::new("not sent")
                        .small()
                        .color(accent_error()),
                );
            }
            TwitchSendState::Received => {}
        }
        if interactive
            && message.send_state == TwitchSendState::Received
            && !message.id.is_empty()
            && !message.id.starts_with("local:")
            && ui
                .add(standard_button("Reply"))
                .on_hover_text("Reply to this message")
                .clicked()
        {
            state.set_reply(message.id.clone(), message.display_name.clone());
        }
    });
}

pub(super) fn username_text(message: &TwitchMessage, alpha: f32) -> egui::RichText {
    let name_color = message
        .name_color
        .map(|[r, g, b]| Color32::from_rgb(r, g, b))
        .unwrap_or_else(|| Color32::from_gray(210))
        .gamma_multiply(alpha);
    egui::RichText::new(format!("{}:", message.display_name))
        .strong()
        .color(name_color)
}

fn paint_composer(
    ui: &mut egui::Ui,
    state: &mut TwitchWidgetState,
    prefs: &TwitchPrefs,
    snapshot: &TwitchSnapshot,
    actions: &mut Vec<TwitchChatAction>,
) {
    if let Some(reply) = state.reply_target.clone() {
        ui.horizontal(|ui| {
            ui.label(meta_text(format!("Replying to {}", reply.display_name)));
            if ui.add(standard_button("Cancel")).clicked() {
                state.reply_target = None;
            }
        });
    }
    let joined = snapshot.connection == TwitchConnectionState::Joined;
    let accepting_input = joined && state.pending_request_id.is_none();
    let response = ui.add_enabled(
        accepting_input,
        singleline_text_edit(&mut state.draft)
            .desired_width(f32::INFINITY)
            .hint_text(if joined {
                "Send a message…"
            } else {
                "Waiting for chat…"
            }),
    );
    if state.draft.chars().count() > TWITCH_MESSAGE_MAX_CHARS {
        state.draft = state.draft.chars().take(TWITCH_MESSAGE_MAX_CHARS).collect();
    }
    let enter_to_send = accepting_input
        && response.lost_focus()
        && ui.input(|input| input.key_pressed(egui::Key::Enter));
    ui.horizontal_wrapped(|ui| {
        let can_send = accepting_input && !state.draft.trim().is_empty();
        let send_clicked = ui.add_enabled(can_send, primary_button("Send")).clicked();
        let send_requested = can_send && (send_clicked || enter_to_send);
        if send_requested && let Some(action) = state.begin_send(snapshot) {
            actions.push(action);
        }
        paint_current_channel_favorite_control(ui, prefs, actions);
        if ui.add(standard_button("Disconnect channel")).clicked() {
            actions.push(TwitchChatAction::ClearChannel);
        }
    });
}

pub fn passive_message_alpha(age: Duration, lifetime: Duration) -> Option<f32> {
    if lifetime.is_zero() || age > lifetime {
        return None;
    }
    let fade_start = lifetime.mul_f32(2.0 / 3.0);
    if age <= fade_start {
        return Some(1.0);
    }
    let fade_duration = lifetime.saturating_sub(fade_start);
    if fade_duration.is_zero() {
        return Some(1.0);
    }
    Some(
        (1.0 - age.saturating_sub(fade_start).as_secs_f32() / fade_duration.as_secs_f32())
            .clamp(0.0, 1.0),
    )
}

pub fn twitch_passive_repaint_after(
    snapshot: &TwitchSnapshot,
    lifetime: Duration,
    now: Instant,
) -> Option<Duration> {
    snapshot
        .messages
        .iter()
        .rev()
        .take(PASSIVE_VISIBLE_MAX)
        .any(|message| {
            now.checked_duration_since(message.received_at)
                .is_some_and(|age| passive_message_alpha(age, lifetime).is_some())
        })
        .then_some(Duration::from_millis(100))
}

fn connection_label(state: &TwitchConnectionState) -> &'static str {
    match state {
        TwitchConnectionState::Inert => "INACTIVE",
        TwitchConnectionState::Disconnected => "DISCONNECTED",
        TwitchConnectionState::Authorizing => "AUTHORIZING",
        TwitchConnectionState::Connecting => "CONNECTING",
        TwitchConnectionState::Joined => "CONNECTED",
        TwitchConnectionState::Reconnecting => "RECONNECTING",
        TwitchConnectionState::Failed(category) => match category {
            TwitchFailureCategory::Authentication => "AUTH FAILED",
            TwitchFailureCategory::AuthorizationExpired => "AUTH EXPIRED",
            TwitchFailureCategory::ChannelUnavailable => "CHANNEL UNAVAILABLE",
            TwitchFailureCategory::Connection => "CONNECTION ERROR",
            TwitchFailureCategory::RateLimited => "RATE LIMITED",
            TwitchFailureCategory::ProviderResponse => "TWITCH ERROR",
            TwitchFailureCategory::CredentialStore => "KEYRING ERROR",
        },
    }
}

fn connection_color(state: &TwitchConnectionState) -> Color32 {
    match state {
        TwitchConnectionState::Joined => accent_ok(),
        TwitchConnectionState::Connecting
        | TwitchConnectionState::Reconnecting
        | TwitchConnectionState::Authorizing => accent_warn(),
        TwitchConnectionState::Failed(_) => accent_error(),
        TwitchConnectionState::Inert | TwitchConnectionState::Disconnected => {
            Color32::from_gray(150)
        }
    }
}
