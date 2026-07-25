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

use super::chrome::{
    ResizeGripOutcome, accent_error, accent_ok, accent_warn, apply_scale, fixed_panel_constraints,
    meta_text, options_menu, panel_frame, report_fixed_panel_size, resize_grip, title_text,
};

const PASSIVE_VISIBLE_MAX: usize = 12;
/// Favorite star (filled). Use a plain glyph; avoid exotic icons that render as tofu.
const FAVORITE_STAR_ON: &str = "★";
const FAVORITE_STAR_OFF: &str = "☆";
const FAVORITE_STAR_YELLOW: Color32 = Color32::from_rgb(255, 200, 60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TwitchChatAction {
    Command(TwitchCommand),
    SetChannel(String),
    ToggleFavorite(String),
    MoveFavorite { from: usize, to: usize },
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
        .interactable(interactive)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            apply_scale(ui, scale);
            panel_frame(transparent_background).show(ui, |ui| {
                fixed_panel_constraints(ui, panel_size, mode, safe_height);
                paint_header(ui, state, prefs, interactive, &mut actions);
                if let Some(message) = &state.message {
                    ui.colored_label(accent_error(), egui::RichText::new(message).small());
                }
                ui.add_space(4.0);
                paint_chat(ui, state, prefs, interactive, panel_size, now, &mut actions);
                let panel_rect = ui.min_rect();
                resize = resize_grip(ui, panel_rect, interactive);
            });
        });

    let measured = response.response.rect.size().max(vec2(1.0, 1.0));
    TwitchChatResponse {
        size: report_fixed_panel_size(panel_size, measured, mode),
        position: response.response.rect.min,
        dragged: response.response.dragged() && !resize.dragging,
        drag_stopped: response.response.drag_stopped() && !resize.dragging && !resize.drag_stopped,
        resize,
        actions,
    }
}

fn paint_header(
    ui: &mut egui::Ui,
    state: &mut TwitchWidgetState,
    prefs: &TwitchPrefs,
    interactive: bool,
    actions: &mut Vec<TwitchChatAction>,
) {
    let snapshot = Arc::clone(&state.snapshot);
    ui.horizontal(|ui| {
        ui.label(title_text("TWITCH CHAT"));
        let channel = snapshot.channel.as_deref().unwrap_or("select a channel");
        ui.label(meta_text(format!("#{channel}")));
        ui.label(
            egui::RichText::new(connection_label(&snapshot.connection))
                .small()
                .color(connection_color(&snapshot.connection)),
        );
        if interactive {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                options_menu(ui, |ui| {
                    paint_options(ui, state, &snapshot, prefs, actions);
                });
                paint_favorite_star(ui, prefs, actions);
            });
        }
    });
}

fn paint_favorite_star(
    ui: &mut egui::Ui,
    prefs: &TwitchPrefs,
    actions: &mut Vec<TwitchChatAction>,
) {
    let Some(channel) = prefs.active_channel.as_deref() else {
        return;
    };
    let is_favorite = prefs.favorites.iter().any(|item| item == channel);
    let can_add = is_favorite || prefs.favorites.len() < TWITCH_FAVORITES_MAX;
    let (glyph, color) = if is_favorite {
        (FAVORITE_STAR_ON, FAVORITE_STAR_YELLOW)
    } else {
        (FAVORITE_STAR_OFF, Color32::from_gray(160))
    };
    let response = ui
        .add_enabled(
            can_add,
            egui::Button::new(egui::RichText::new(glyph).size(14.0).color(color)).frame(false),
        )
        .on_hover_text(if is_favorite {
            "Remove from favorites"
        } else if can_add {
            "Add to favorites"
        } else {
            "Favorite channel limit reached"
        });
    if response.clicked() {
        actions.push(TwitchChatAction::ToggleFavorite(channel.to_owned()));
    }
}

fn paint_options(
    ui: &mut egui::Ui,
    state: &mut TwitchWidgetState,
    snapshot: &TwitchSnapshot,
    prefs: &TwitchPrefs,
    actions: &mut Vec<TwitchChatAction>,
) {
    ui.set_min_width(280.0);
    ui.label(egui::RichText::new("TWITCH ACCOUNT").small().strong());
    if !snapshot.client_configured {
        ui.colored_label(accent_warn(), "Twitch is not configured in this build yet.");
    } else if let Some(login) = &snapshot.authenticated_login {
        ui.label(format!("Signed in as {login}"));
        if !snapshot.credentials_persisted {
            ui.colored_label(
                accent_warn(),
                "Secret Service unavailable — reconnect required after restart.",
            );
        }
        let joined = snapshot.connection == TwitchConnectionState::Joined
            || snapshot.connection == TwitchConnectionState::Connecting
            || snapshot.connection == TwitchConnectionState::Reconnecting;
        if !joined && ui.button("Reconnect chat").clicked() {
            actions.push(TwitchChatAction::Command(TwitchCommand::Reconnect));
        }
        if joined && ui.button("Disconnect chat").clicked() {
            // Soft disconnect: close chat while keeping the stored session.
            actions.push(TwitchChatAction::Command(TwitchCommand::Disconnect));
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
    } else if let Some(authorization) = &snapshot.authorization {
        ui.label("Enter this code on Twitch:");
        ui.monospace(
            egui::RichText::new(&authorization.user_code)
                .size(18.0)
                .strong(),
        );
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !state.verification_opening,
                    egui::Button::new("Open Twitch"),
                )
                .clicked()
            {
                actions.push(TwitchChatAction::OpenVerification(
                    authorization.verification_uri.clone(),
                ));
            }
            if ui.button("Cancel").clicked() {
                actions.push(TwitchChatAction::Command(
                    TwitchCommand::CancelAuthentication,
                ));
            }
        });
    } else if ui.button("Connect Twitch").clicked() {
        actions.push(TwitchChatAction::Command(
            TwitchCommand::BeginAuthentication,
        ));
    }

    ui.separator();
    ui.label(egui::RichText::new("CHANNEL").small().strong());
    if state.channel_draft.chars().count() > TWITCH_CHANNEL_MAX_CHARS + 1 {
        state.channel_draft = state
            .channel_draft
            .chars()
            .take(TWITCH_CHANNEL_MAX_CHARS + 1)
            .collect();
    }
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.channel_draft)
                .desired_width(160.0)
                .hint_text("channel"),
        );
        // Normalize after the edit so typing this frame enables Join correctly.
        let normalized = normalize_twitch_channel(&state.channel_draft).ok();
        let apply_requested = ui
            .add_enabled(normalized.is_some(), egui::Button::new("Join chat"))
            .clicked()
            || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if apply_requested && let Some(channel) = normalized {
            actions.push(TwitchChatAction::SetChannel(channel));
            ui.close();
        }
    });

    if !prefs.favorites.is_empty() {
        paint_favorites_submenu(ui, prefs, actions);
    }

    ui.separator();
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

fn paint_favorites_submenu(
    ui: &mut egui::Ui,
    prefs: &TwitchPrefs,
    actions: &mut Vec<TwitchChatAction>,
) {
    let count = prefs.favorites.len();
    let label = format!("Favorites ({count})");
    // Nested menu keeps the main options panel short; reorder only when useful.
    ui.menu_button(label, |ui| {
        ui.set_min_width(200.0);
        for (index, favorite) in prefs.favorites.iter().enumerate() {
            ui.horizontal(|ui| {
                let selected = prefs.active_channel.as_deref() == Some(favorite.as_str());
                if ui
                    .selectable_label(selected, format!("#{favorite}"))
                    .clicked()
                {
                    actions.push(TwitchChatAction::SetChannel(favorite.clone()));
                    ui.close();
                }
                if ui
                    .small_button(FAVORITE_STAR_ON)
                    .on_hover_text("Remove favorite")
                    .clicked()
                {
                    actions.push(TwitchChatAction::ToggleFavorite(favorite.clone()));
                }
                // Only draw move controls that can actually move — disabled
                // small buttons render as empty tofu squares in this theme.
                if index > 0 && ui.small_button("Up").on_hover_text("Move up").clicked() {
                    actions.push(TwitchChatAction::MoveFavorite {
                        from: index,
                        to: index - 1,
                    });
                }
                if index + 1 < count && ui.small_button("Down").on_hover_text("Move down").clicked()
                {
                    actions.push(TwitchChatAction::MoveFavorite {
                        from: index,
                        to: index + 1,
                    });
                }
            });
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
    if snapshot.channel.is_none() {
        ui.label(meta_text(
            "Choose a public Twitch channel in the widget options.",
        ));
        return;
    }
    if !snapshot.client_configured {
        ui.label(meta_text(
            "This build needs the PlayerVox Twitch Client ID before chat can connect.",
        ));
        return;
    }
    if snapshot.authenticated_login.is_none() {
        ui.label(meta_text(if snapshot.credentials_available {
            "Twitch is temporarily unavailable. Retrying automatically."
        } else {
            "Connect Twitch from the widget options to read and send chat."
        }));
        return;
    }

    let composer_height = if interactive { 72.0 } else { 0.0 };
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
                .button(format!("{} new messages ↓", state.unread_count))
                .clicked()
        {
            state.return_to_latest();
        }
        paint_composer(ui, state, &snapshot, actions);
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
                .color(Color32::from_gray(140).gamma_multiply(alpha)),
        );
    }
    // Single label avoids horizontal_wrapped empty chips before the name.
    let body_color = Color32::from_gray(235).gamma_multiply(alpha);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
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
                .small_button("Reply")
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
    snapshot: &TwitchSnapshot,
    actions: &mut Vec<TwitchChatAction>,
) {
    if let Some(reply) = state.reply_target.clone() {
        ui.horizontal(|ui| {
            ui.label(meta_text(format!("Replying to {}", reply.display_name)));
            if ui.small_button("Cancel").clicked() {
                state.reply_target = None;
            }
        });
    }
    let joined = snapshot.connection == TwitchConnectionState::Joined;
    let accepting_input = joined && state.pending_request_id.is_none();
    let response = ui.add_enabled(
        accepting_input,
        egui::TextEdit::singleline(&mut state.draft)
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
    // Single-line TextEdit loses focus on Enter, so lost_focus (not has_focus)
    // is the reliable signal — same pattern as the channel field.
    let enter_to_send = accepting_input
        && response.lost_focus()
        && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let send_requested = accepting_input
        && !state.draft.trim().is_empty()
        && (ui.button("Send").clicked() || enter_to_send);
    if send_requested && let Some(action) = state.begin_send(snapshot) {
        actions.push(action);
    }
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
