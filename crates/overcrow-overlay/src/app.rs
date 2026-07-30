use std::{
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    branding::{BrandAssets, BrandSize, install_fonts, paint_brand},
    media::{MediaClient, MediaSnapshot},
    notes::{LocalNotesRepository, NotesCommand, NotesService},
    preferences::{OverlayPreferences, PreferenceStore},
    runtime::{ProviderReadiness, SnapshotClient, SnapshotUpdate},
    session_clock::SessionClock,
    twitch::{
        client::{TwitchClient, TwitchGate},
        emotes::TwitchEmotes,
        http::validate_verification_uri,
        model::{TwitchCommand, TwitchConnectionState, TwitchSnapshot},
        prefs::{TwitchPrefsSaveOutcome, TwitchPrefsSaver},
    },
    warframe::{WarframeActionBatch, WarframeController, is_warframe_active},
    widgets::{
        ACCENT, CatalogAction, CatalogActionOutcome, CatalogLayout, ManualStopwatchClock,
        NotesWidgetState, PANEL_FILL, PANEL_STROKE_STRONG, TEXT_MUTED, TwitchChatAction,
        TwitchWidgetState, WidgetManager, apply_catalog_action, catalog_visible, install_theme,
        manual_stopwatch_repaint_after, notes_action_allowed, paint_catalog, paint_fissure_options,
        paint_gated_options, paint_invasion_options, paint_profile_options, paint_sortie_options,
        paint_status_options, paint_twitch_options, persist_profile_change,
        route_manual_stopwatch_action, session_repaint_after as stopwatch_repaint_after,
        status_pill, twitch_passive_repaint_after,
    },
};
use eframe::egui;
use overcrow_config::{TWITCH_FAVORITES_MAX, TwitchPrefs, TwitchPrefsStore, WidgetId};
use overcrow_logging::EventLogger;
use overcrow_protocol::{CoreSnapshot, OverlayMode};

pub const APP_ID: &str = "io.github.overcrow.Overlay";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const LICENSE_ID: &str = "AGPL-3.0-only";
const NOTICE_TEXT: &str = include_str!("../../../NOTICE");
const SOURCE_REPOSITORY_URL: &str = "https://github.com/Valhallab/PlayerVox-OverCrow";
const WIDGET_MARGIN: f32 = 24.0;

fn wall_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ViewportUpdate {
    mouse_passthrough: bool,
    position: Option<[f32; 2]>,
    size: Option<[f32; 2]>,
}

impl ViewportUpdate {
    fn from_snapshot(snapshot: &CoreSnapshot, core_authority: bool, pixels_per_point: f32) -> Self {
        if !core_authority {
            return Self {
                mouse_passthrough: true,
                position: None,
                size: None,
            };
        }
        let (position, size) = snapshot
            .active_game
            .as_ref()
            .filter(|game| game.backend == "x11")
            .map_or((None, None), |game| {
                // EWMH geometry is expressed in physical X11 pixels, while
                // egui viewport commands are converted from logical points.
                let scale = valid_pixels_per_point(pixels_per_point);
                (
                    Some([game.rect.x as f32 / scale, game.rect.y as f32 / scale]),
                    Some([
                        game.rect.width as f32 / scale,
                        game.rect.height as f32 / scale,
                    ]),
                )
            });
        Self {
            mouse_passthrough: snapshot.overlay_mode == OverlayMode::Passive
                || snapshot.active_game.is_none(),
            position,
            size,
        }
    }
}

fn viewport_update_changed(
    previous: &CoreSnapshot,
    core_authority: bool,
    pixels_per_point: f32,
    update: &ViewportUpdate,
) -> bool {
    ViewportUpdate::from_snapshot(previous, core_authority, pixels_per_point) != *update
}

fn valid_pixels_per_point(pixels_per_point: f32) -> f32 {
    if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
        pixels_per_point
    } else {
        1.0
    }
}

fn x11_scale_changed(
    applied: Option<f32>,
    current: f32,
    snapshot: &CoreSnapshot,
    core_authority: bool,
) -> bool {
    core_authority
        && snapshot
            .active_game
            .as_ref()
            .is_some_and(|game| game.backend == "x11")
        && applied != Some(valid_pixels_per_point(current))
}

fn x11_should_request_focus(x11_session: bool, mode_event: Option<OverlayMode>) -> bool {
    x11_session && mode_event == Some(OverlayMode::Interactive)
}

fn schedule_wayland_input_region_commit(context: &egui::Context, x11_session: bool) {
    if !x11_session {
        // eframe processes viewport commands after swapping the current frame.
        // Mutter applies the pending Wayland input region on the next surface
        // commit, so schedule one bounded follow-up frame.
        context.request_repaint_after(Duration::from_millis(1));
    }
}

fn authoritative_snapshot(snapshot: &CoreSnapshot, core_authority: bool) -> CoreSnapshot {
    if core_authority {
        snapshot.clone()
    } else {
        CoreSnapshot::default()
    }
}

fn confirmed_mode_event(
    previous: OverlayMode,
    passive_pending: bool,
    update: &SnapshotUpdate,
) -> Option<OverlayMode> {
    if !update.is_confirmed() || (passive_pending && !update.passive_confirmed) {
        return None;
    }
    (previous != update.snapshot.overlay_mode).then_some(update.snapshot.overlay_mode)
}

#[derive(Debug, Default)]
struct OverlayState {
    snapshot: CoreSnapshot,
    passive_pending: bool,
}

impl OverlayState {
    #[cfg(test)]
    fn from_snapshot(snapshot: CoreSnapshot) -> Self {
        Self {
            snapshot,
            passive_pending: false,
        }
    }

    fn begin_passive_request(&mut self) {
        self.passive_pending = true;
    }

    fn apply_snapshot(
        &mut self,
        update: SnapshotUpdate,
        core_authority: bool,
        pixels_per_point: f32,
    ) -> ViewportUpdate {
        if update.passive_confirmed {
            self.passive_pending = false;
        }
        if self.passive_pending {
            return ViewportUpdate::from_snapshot(&self.snapshot, core_authority, pixels_per_point);
        }
        let viewport =
            ViewportUpdate::from_snapshot(&update.snapshot, core_authority, pixels_per_point);
        self.snapshot = update.snapshot;
        viewport
    }

    fn snapshot(&self) -> &CoreSnapshot {
        &self.snapshot
    }

    #[cfg(test)]
    fn passive_pending(&self) -> bool {
        self.passive_pending
    }
}

pub fn viewport_builder(x11_session: bool) -> egui::ViewportBuilder {
    let viewport = egui::ViewportBuilder::default()
        .with_title("OverCrow")
        .with_app_id(APP_ID)
        .with_transparent(true)
        .with_decorations(false)
        .with_resizable(true)
        .with_mouse_passthrough(true);

    if x11_session {
        viewport.with_always_on_top()
    } else {
        viewport
    }
}

pub fn is_x11_session() -> bool {
    if let Ok(session_type) = std::env::var("XDG_SESSION_TYPE") {
        return session_type.eq_ignore_ascii_case("x11");
    }
    std::env::var_os("DISPLAY").is_some() && std::env::var_os("WAYLAND_DISPLAY").is_none()
}

pub struct OverlayApp {
    logger: EventLogger,
    client: SnapshotClient,
    state: OverlayState,
    session_clock: SessionClock,
    manual_stopwatch_clock: ManualStopwatchClock,
    media_client: MediaClient,
    media_snapshot: Arc<MediaSnapshot>,
    media_revision: u64,
    media_readiness: ProviderReadiness,
    notes_service: NotesService,
    notes_state: NotesWidgetState,
    twitch_client: TwitchClient,
    twitch_state: TwitchWidgetState,
    twitch_prefs: TwitchPrefs,
    twitch_prefs_saver: TwitchPrefsSaver,
    twitch_verification: VerificationLauncher,
    warframe: WarframeController,
    preferences: OverlayPreferences,
    preference_store: PreferenceStore,
    widgets: WidgetManager,
    brand: BrandAssets,
    about_open: bool,
    core_authority: bool,
    x11_session: bool,
    x11_viewport_pixels_per_point: Option<f32>,
}

impl OverlayApp {
    pub fn new(
        creation_context: &eframe::CreationContext<'_>,
        logger: EventLogger,
        x11_session: bool,
    ) -> Self {
        install_fonts(&creation_context.egui_ctx);
        install_theme(&creation_context.egui_ctx);
        let repaint_context = creation_context.egui_ctx.clone();
        let client_repaint_context = repaint_context.clone();
        let media_repaint_context = repaint_context.clone();
        let notes_repaint_context = repaint_context.clone();
        let twitch_repaint_context = repaint_context.clone();
        let twitch_emote_repaint_context = repaint_context.clone();
        let twitch_settings_repaint_context = repaint_context.clone();
        let media_readiness = ProviderReadiness::default();
        let media_callback_readiness = media_readiness.clone();
        let client = SnapshotClient::spawn(logger.clone(), move || {
            client_repaint_context.request_repaint();
        });
        let media_client = MediaClient::spawn(logger.clone(), move || {
            media_callback_readiness.mark_media();
            media_repaint_context.request_repaint();
        });
        let notes_service = NotesService::spawn_with_logger(
            LocalNotesRepository::from_environment(),
            logger.clone(),
            move || {
                notes_repaint_context.request_repaint();
            },
        );
        let twitch_client = TwitchClient::spawn(logger.clone(), move || {
            twitch_repaint_context.request_repaint();
        });
        let twitch_emotes = TwitchEmotes::spawn(logger.clone(), move || {
            twitch_emote_repaint_context.request_repaint();
        });
        creation_context
            .egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));
        let preference_store = PreferenceStore::from_environment();
        let preference_load = preference_store.load();
        if let Some(warning) = &preference_load.warning {
            eprintln!("OverCrow widget settings rejected; using defaults: {warning}");
            logger.warn(
                "widget_settings_load_failed",
                format_args!("affected_widgets=all category=validation"),
            );
        }
        let twitch_prefs_store = TwitchPrefsStore::from_environment();
        let twitch_prefs_load = twitch_prefs_store.load();
        if twitch_prefs_load.warning.is_some() {
            eprintln!("OverCrow Twitch settings rejected; using defaults");
            logger.warn(
                "widget_settings_load_failed",
                format_args!("widget=twitch_chat category=validation"),
            );
        }
        let twitch_prefs_saver = TwitchPrefsSaver::spawn(twitch_prefs_store, move || {
            twitch_settings_repaint_context.request_repaint();
        });
        Self {
            logger: logger.clone(),
            client,
            state: OverlayState::default(),
            session_clock: SessionClock::default(),
            manual_stopwatch_clock: ManualStopwatchClock::default(),
            media_client,
            media_snapshot: Arc::new(MediaSnapshot::default()),
            media_revision: 0,
            media_readiness,
            notes_service,
            notes_state: NotesWidgetState::default(),
            twitch_client,
            twitch_state: TwitchWidgetState::with_emotes(twitch_emotes),
            twitch_prefs: twitch_prefs_load.prefs,
            twitch_prefs_saver,
            twitch_verification: VerificationLauncher::default(),
            warframe: WarframeController::new(&creation_context.egui_ctx, logger.clone()),
            preferences: preference_load.profile,
            preference_store,
            widgets: WidgetManager::default(),
            brand: BrandAssets::default(),
            about_open: false,
            core_authority: false,
            x11_session,
            x11_viewport_pixels_per_point: None,
        }
    }

    fn apply_snapshot(&mut self, context: &egui::Context, snapshot: SnapshotUpdate) {
        let previous = self.state.snapshot().clone();
        let mode_event =
            confirmed_mode_event(previous.overlay_mode, self.state.passive_pending, &snapshot);
        let pixels_per_point = context.pixels_per_point();
        let update = self
            .state
            .apply_snapshot(snapshot, self.core_authority, pixels_per_point);
        if let Some(mode) = mode_event {
            self.logger
                .info("overlay_mode_confirmed", format_args!("mode={mode:?}"));
        }
        let received_at = Instant::now();
        self.session_clock
            .sync(self.state.snapshot().session_elapsed_ms, received_at);
        self.manual_stopwatch_clock
            .sync(self.state.snapshot().manual_stopwatch, received_at);
        self.client
            .set_manual_stopwatch_running(self.manual_stopwatch_clock.running());
        if viewport_update_changed(&previous, self.core_authority, pixels_per_point, &update) {
            self.apply_viewport_update(context, update, pixels_per_point);
        }

        if x11_should_request_focus(self.x11_session, mode_event) {
            context.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn sync_core_authority(&mut self, context: &egui::Context) {
        let authority = self.client.has_authority();
        if authority == self.core_authority {
            return;
        }
        let pixels_per_point = context.pixels_per_point();
        let previous = ViewportUpdate::from_snapshot(
            self.state.snapshot(),
            self.core_authority,
            pixels_per_point,
        );
        self.core_authority = authority;
        let update = ViewportUpdate::from_snapshot(
            self.state.snapshot(),
            self.core_authority,
            pixels_per_point,
        );
        if previous != update {
            self.apply_viewport_update(context, update, pixels_per_point);
        }
    }

    fn sync_x11_scale(&mut self, context: &egui::Context) {
        let pixels_per_point = context.pixels_per_point();
        if !x11_scale_changed(
            self.x11_viewport_pixels_per_point,
            pixels_per_point,
            self.state.snapshot(),
            self.core_authority,
        ) {
            return;
        }
        let update = ViewportUpdate::from_snapshot(
            self.state.snapshot(),
            self.core_authority,
            pixels_per_point,
        );
        self.apply_viewport_update(context, update, pixels_per_point);
    }

    fn apply_viewport_update(
        &mut self,
        context: &egui::Context,
        update: ViewportUpdate,
        pixels_per_point: f32,
    ) {
        self.x11_viewport_pixels_per_point = update
            .position
            .zip(update.size)
            .map(|_| valid_pixels_per_point(pixels_per_point));
        context.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(
            update.mouse_passthrough,
        ));
        schedule_wayland_input_region_commit(context, self.x11_session);
        if self.x11_session {
            context.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::AlwaysOnTop,
            ));
        }
        if let Some([x, y]) = update.position {
            context.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x, y)));
        }
        if let Some([width, height]) = update.size {
            context.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
        }
    }

    fn request_passive(&mut self, context: &egui::Context) {
        if self.state.snapshot().overlay_mode != OverlayMode::Interactive {
            return;
        }
        self.state.begin_passive_request();
        // Apply click-through immediately so the next pointer event cannot
        // land on the full-screen surface while Core still reports Interactive.
        context.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));
        self.logger
            .info("passive_requested", format_args!("source=overlay"));
        self.warframe.sync(
            context,
            self.state.snapshot(),
            &self.preferences,
            Instant::now(),
            wall_secs(),
        );
        self.client.request_passive();
    }

    fn apply_catalog_action(&mut self, context: &egui::Context, action: CatalogAction) {
        let widget_id = action.widget_id();
        let outcome = apply_catalog_action(&mut self.preferences, action, |candidate| {
            self.preference_store.save(candidate)
        });
        let published = !matches!(outcome, CatalogActionOutcome::RolledBack { .. });
        if published {
            match action {
                CatalogAction::ResetPosition(id) => self.widgets.clear_runtime_position(id),
                CatalogAction::ResetSize(id) => self.widgets.clear_runtime_size(id),
                _ => {}
            }
        }
        log_catalog_settings_outcome(&self.logger, widget_id, &outcome);
        self.warframe.sync(
            context,
            self.state.snapshot(),
            &self.preferences,
            Instant::now(),
            wall_secs(),
        );
        let client = &self.client;
        handle_catalog_outcome(&mut self.widgets, outcome, || {
            client.reload_widget_settings();
        });
        self.sync_twitch_gate();
        context.request_repaint();
    }

    fn commit_geometry_preferences(&mut self, previous: OverlayPreferences) {
        let outcome = persist_profile_change(&mut self.preferences, previous, |candidate| {
            self.preference_store.save(candidate)
        });
        let rolled_back = matches!(outcome, CatalogActionOutcome::RolledBack { .. });
        log_settings_outcome(&self.logger, None, &outcome);
        handle_catalog_outcome(&mut self.widgets, outcome, || {});
        if rolled_back {
            self.widgets.clear_runtime_geometry();
        }
    }

    fn sync_twitch_gate(&mut self) {
        let gate = twitch_gate(
            self.client.has_authority(),
            self.state.snapshot().active_game.is_some(),
            self.preferences.settings(WidgetId::TwitchChat).enabled,
            self.twitch_prefs.active_channel.clone(),
        );
        let emotes_enabled = twitch_emotes_allowed(&gate, self.twitch_state.snapshot());
        self.twitch_client.set_gate(gate);
        self.twitch_state.set_emotes_enabled(emotes_enabled);
    }

    fn commit_twitch_prefs(&mut self, candidate: TwitchPrefs) -> bool {
        match self.twitch_prefs_saver.try_save(candidate) {
            Ok(()) => true,
            Err(error) => {
                self.twitch_state.set_message(Some(
                    if matches!(error, crate::twitch::prefs::TwitchPrefsSaveFailure::Busy) {
                        "Twitch settings are still saving. Try again.".to_owned()
                    } else {
                        "Could not save Twitch widget settings.".to_owned()
                    },
                ));
                self.logger.warn(
                    "widget_settings_save_failed",
                    format_args!("widget=twitch_chat category={}", error.category()),
                );
                false
            }
        }
    }

    fn apply_twitch_prefs_save(&mut self, outcome: &TwitchPrefsSaveOutcome) {
        match outcome {
            TwitchPrefsSaveOutcome::Durable(candidate) => {
                self.twitch_prefs.clone_from(candidate);
                self.twitch_state.set_message(None);
            }
            TwitchPrefsSaveOutcome::CommittedWithWarning(candidate) => {
                self.twitch_prefs.clone_from(candidate);
                self.twitch_state.set_message(Some(
                    "Saved, but storage durability could not be confirmed.".to_owned(),
                ));
                self.logger.warn(
                    "widget_settings_save_failed",
                    format_args!("widget=twitch_chat category=durability"),
                );
            }
            TwitchPrefsSaveOutcome::RolledBack(error) => {
                self.twitch_state
                    .set_message(Some("Could not save Twitch widget settings.".to_owned()));
                self.logger.warn(
                    "widget_settings_save_failed",
                    format_args!("widget=twitch_chat category={}", error.category()),
                );
            }
        }
    }

    fn dispatch_twitch_action(&mut self, action: TwitchChatAction) {
        if self.state.snapshot().overlay_mode != OverlayMode::Interactive
            || self.state.snapshot().active_game.is_none()
        {
            return;
        }

        match action {
            TwitchChatAction::Command(TwitchCommand::SendMessage {
                request_id,
                generation,
                channel,
                text,
                reply_to,
            }) => {
                if !self.twitch_client.try_send(TwitchCommand::SendMessage {
                    request_id,
                    generation,
                    channel,
                    text,
                    reply_to,
                }) {
                    self.twitch_state.mark_send_rejected(request_id);
                }
            }
            TwitchChatAction::Command(command) => {
                if !self.twitch_client.try_send(command) {
                    self.twitch_state
                        .set_message(Some("Twitch is busy. Try again.".to_owned()));
                }
            }
            TwitchChatAction::SetChannel(channel) => {
                let mut candidate = self.twitch_prefs.clone();
                candidate.active_channel = Some(channel.clone());
                if self.commit_twitch_prefs(candidate) {
                    self.twitch_state.set_channel_draft(&channel);
                    self.sync_twitch_gate();
                }
            }
            TwitchChatAction::ClearChannel => {
                let mut candidate = self.twitch_prefs.clone();
                candidate.active_channel = None;
                if self.commit_twitch_prefs(candidate) {
                    self.twitch_state.set_channel_draft("");
                }
            }
            TwitchChatAction::ToggleFavorite(channel) => {
                let mut candidate = self.twitch_prefs.clone();
                if let Some(index) = candidate
                    .favorites
                    .iter()
                    .position(|favorite| favorite == &channel)
                {
                    candidate.favorites.remove(index);
                } else if candidate.favorites.len() < TWITCH_FAVORITES_MAX {
                    candidate.favorites.push(channel);
                } else {
                    self.twitch_state
                        .set_message(Some("Favorite channel limit reached.".to_owned()));
                    return;
                }
                self.commit_twitch_prefs(candidate);
            }
            TwitchChatAction::SetPassiveLifetime(seconds) => {
                let mut candidate = self.twitch_prefs.clone();
                candidate.passive_lifetime_secs = seconds;
                self.commit_twitch_prefs(candidate);
            }
            TwitchChatAction::OpenVerification(uri) => {
                if !self.twitch_verification.open(uri) {
                    self.twitch_state
                        .set_message(Some("Could not open Twitch in your browser.".to_owned()));
                }
            }
        }
    }
}

fn twitch_gate(
    core_authority: bool,
    active_game_authorized: bool,
    widget_enabled: bool,
    channel: Option<String>,
) -> TwitchGate {
    TwitchGate {
        lifecycle_enabled: core_authority,
        active_game_authorized,
        widget_enabled,
        channel,
    }
}

fn twitch_emotes_allowed(gate: &TwitchGate, snapshot: &TwitchSnapshot) -> bool {
    gate.lifecycle_enabled
        && gate.active_game_authorized
        && gate.widget_enabled
        && gate.channel.is_some()
        && gate.channel == snapshot.channel
        && snapshot.connection == TwitchConnectionState::Joined
}

#[derive(Clone, Default)]
struct LaunchGate(Arc<AtomicBool>);

impl LaunchGate {
    fn try_acquire(&self) -> bool {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn release(&self) {
        self.0.store(false, Ordering::Release);
    }

    fn active(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

struct VerificationLauncher {
    gate: LaunchGate,
    results: Receiver<bool>,
    result_sender: mpsc::SyncSender<bool>,
}

impl Default for VerificationLauncher {
    fn default() -> Self {
        let (result_sender, results) = mpsc::sync_channel(1);
        Self {
            gate: LaunchGate::default(),
            results,
            result_sender,
        }
    }
}

impl VerificationLauncher {
    fn open(&self, uri: String) -> bool {
        if validate_verification_uri(&uri).is_err() || !self.gate.try_acquire() {
            return false;
        }
        let gate = self.gate.clone();
        let results = self.result_sender.clone();
        let spawn = thread::Builder::new()
            .name("overcrow-open-twitch".to_owned())
            .spawn(move || {
                let succeeded = run_xdg_open(uri);
                gate.release();
                let _ = results.try_send(succeeded);
            });
        if spawn.is_err() {
            self.gate.release();
            return false;
        }
        true
    }

    fn active(&self) -> bool {
        self.gate.active()
    }

    fn take_result(&self) -> Option<bool> {
        self.results.try_recv().ok()
    }
}

fn run_xdg_open(uri: String) -> bool {
    let Ok(mut child) = Command::new("/usr/bin/xdg-open")
        .arg(uri)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(5))
        .unwrap_or_else(Instant::now);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Err(_) => return false,
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn settings_failure_target(widget_id: Option<WidgetId>) -> &'static str {
    match widget_id {
        Some(WidgetId::Session) => "widget=session",
        Some(WidgetId::Clock) => "widget=clock",
        Some(WidgetId::Performance) => "widget=performance",
        Some(WidgetId::ManualStopwatch) => "widget=manual_stopwatch",
        Some(WidgetId::Media) => "widget=media",
        Some(WidgetId::Notes) => "widget=notes",
        Some(WidgetId::WarframeStatus) => "widget=warframe_status",
        Some(WidgetId::WarframeFissures) => "widget=warframe_fissures",
        Some(WidgetId::WarframeMarket) => "widget=warframe_market",
        Some(WidgetId::WarframeSortie) => "widget=warframe_sortie",
        Some(WidgetId::WarframeInvasions) => "widget=warframe_invasions",
        Some(WidgetId::TwitchChat) => "widget=twitch_chat",
        None => "affected_widgets=layout",
    }
}

fn paint_control_notices(
    ui: &mut egui::Ui,
    warframe: Option<&str>,
    catalog: Option<&str>,
    twitch: Option<&str>,
) {
    for message in [warframe, catalog, twitch].into_iter().flatten() {
        ui.add_space(4.0);
        ui.colored_label(egui::Color32::from_rgb(251, 113, 133), message);
    }
}

fn log_catalog_settings_outcome(
    logger: &EventLogger,
    widget_id: WidgetId,
    outcome: &CatalogActionOutcome,
) {
    log_settings_outcome(logger, Some(widget_id), outcome);
}

fn log_settings_outcome(
    logger: &EventLogger,
    widget_id: Option<WidgetId>,
    outcome: &CatalogActionOutcome,
) {
    let category = match outcome {
        CatalogActionOutcome::Durable(_) => return,
        CatalogActionOutcome::CommittedWithWarning { .. } => "durability",
        CatalogActionOutcome::RolledBack { category, .. } => category.name(),
    };
    logger.warn(
        "widget_settings_save_failed",
        format_args!("{} category={category}", settings_failure_target(widget_id)),
    );
}

fn handle_catalog_outcome(
    widgets: &mut WidgetManager,
    outcome: CatalogActionOutcome,
    request_reload: impl FnOnce(),
) {
    let commit = match outcome {
        CatalogActionOutcome::Durable(commit) => {
            widgets.set_catalog_message(None);
            Some(commit)
        }
        CatalogActionOutcome::CommittedWithWarning { commit, message } => {
            widgets.set_catalog_message(Some(message));
            Some(commit)
        }
        CatalogActionOutcome::RolledBack { message, .. } => {
            widgets.set_catalog_message(Some(message));
            None
        }
    };

    if commit.is_some_and(|commit| commit.reload_widget_settings) {
        request_reload();
    }
}

trait ManualStopwatchCommandClient {
    fn toggle_manual_stopwatch(&self);
    fn reset_manual_stopwatch(&self);
}

impl ManualStopwatchCommandClient for SnapshotClient {
    fn toggle_manual_stopwatch(&self) {
        SnapshotClient::toggle_manual_stopwatch(self);
    }

    fn reset_manual_stopwatch(&self) {
        SnapshotClient::reset_manual_stopwatch(self);
    }
}

fn dispatch_manual_stopwatch_action(
    client: &impl ManualStopwatchCommandClient,
    clock: &mut ManualStopwatchClock,
    mode: OverlayMode,
    action: Option<crate::widgets::ManualStopwatchAction>,
    now: Instant,
) {
    // Freeze/start locally first so the display cannot overshoot while Core answers.
    match action {
        Some(crate::widgets::ManualStopwatchAction::Toggle) if mode == OverlayMode::Interactive => {
            clock.apply_local_toggle(now);
        }
        Some(crate::widgets::ManualStopwatchAction::Reset) if mode == OverlayMode::Interactive => {
            clock.apply_local_reset(now);
        }
        _ => {}
    }
    route_manual_stopwatch_action(
        mode,
        action,
        || client.toggle_manual_stopwatch(),
        || client.reset_manual_stopwatch(),
    );
}

trait NotesCommandClient {
    fn send_notes(&self, command: NotesCommand) -> Result<(), crate::notes::NotesError>;
}

impl NotesCommandClient for NotesService {
    fn send_notes(&self, command: NotesCommand) -> Result<(), crate::notes::NotesError> {
        self.send(command)
    }
}

fn dispatch_notes_action(
    client: &impl NotesCommandClient,
    state: &mut NotesWidgetState,
    mode: OverlayMode,
    command: NotesCommand,
) {
    if !notes_action_allowed(mode, &command) {
        return;
    }
    let previous = state.clone();
    if let Err(error) = state.accept(&command) {
        state.set_error(error);
        return;
    }
    if let Err(error) = client.send_notes(command) {
        *state = previous;
        state.set_error(error);
    }
}

impl eframe::App for OverlayApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let wall_secs = wall_secs();
        if let Some(snapshot) = self.client.take_latest() {
            self.apply_snapshot(context, snapshot);
        }
        self.sync_core_authority(context);
        self.sync_x11_scale(context);
        let authoritative = authoritative_snapshot(self.state.snapshot(), self.core_authority);
        if self.media_readiness.take().media()
            && let Some(snapshot) = self.media_client.take_latest()
        {
            self.media_revision = snapshot.revision;
            self.media_snapshot = snapshot.value;
        }
        if let Some(update) = self.notes_service.take_latest() {
            self.notes_state.apply_update(update);
        }
        if let Some(snapshot) = self.twitch_client.take_latest() {
            self.twitch_state
                .apply_snapshot(snapshot.revision, snapshot.value);
        }
        self.twitch_state.poll_emotes(context, now);
        if let Some(result) = self.twitch_prefs_saver.take_latest() {
            self.apply_twitch_prefs_save(result.value.as_ref());
        }
        if self.twitch_verification.take_result() == Some(false) {
            self.twitch_state
                .set_message(Some("Could not open Twitch in your browser.".to_owned()));
        }
        self.twitch_state
            .set_verification_opening(self.twitch_verification.active());
        self.sync_twitch_gate();
        self.warframe
            .sync(context, &authoritative, &self.preferences, now, wall_secs);
        if self.core_authority && context.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.request_passive(context);
        }

        let next_repaint = [
            stopwatch_repaint_after(&authoritative, &self.preferences, &self.session_clock, now),
            manual_stopwatch_repaint_after(
                &authoritative,
                &self.preferences,
                &self.manual_stopwatch_clock,
                now,
            ),
        ]
        .into_iter()
        .flatten()
        .min();
        if let Some(delay) = next_repaint {
            context.request_repaint_after(delay);
        }
        if authoritative.overlay_mode == OverlayMode::Passive
            && authoritative.active_game.is_some()
            && self.preferences.settings(WidgetId::TwitchChat).enabled
            && self
                .preferences
                .settings(WidgetId::TwitchChat)
                .show_in_passive
            && let Some(delay) = twitch_passive_repaint_after(
                self.twitch_state.snapshot(),
                Duration::from_secs(self.twitch_prefs.passive_lifetime_secs.into()),
                now,
            )
        {
            context.request_repaint_after(delay);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.core_authority {
            self.about_open = false;
            self.widgets.sync_interaction_state(
                OverlayMode::Passive,
                false,
                ui.input(|input| input.pointer.primary_down()),
            );
            return;
        }
        if !controls_visible(self.state.snapshot()) {
            self.about_open = false;
        }
        if self.state.snapshot().active_game.is_none() {
            self.widgets.sync_interaction_state(
                self.state.snapshot().overlay_mode,
                false,
                ui.input(|input| input.pointer.primary_down()),
            );
            return;
        }
        self.widgets.begin_widget_frame();
        let preferences_before_geometry = self.preferences.clone();
        let widget_input_allowed =
            widget_actions_allowed(self.widgets.catalog_open(), self.about_open);
        self.widgets.set_interaction_enabled(widget_input_allowed);

        if let Some(scrim) = interactive_scrim(self.state.snapshot()) {
            ui.painter().rect_filled(ui.max_rect(), 0.0, scrim);
        }

        let mut save_requested = self.widgets.render_session(
            ui,
            self.state.snapshot(),
            &mut self.preferences,
            &self.session_clock,
            Instant::now(),
            WIDGET_MARGIN,
        );
        save_requested |= self.widgets.render_clock(
            ui,
            self.state.snapshot(),
            &mut self.preferences,
            WIDGET_MARGIN,
        );
        save_requested |= self.widgets.render_performance(
            ui,
            self.state.snapshot(),
            &mut self.preferences,
            WIDGET_MARGIN,
        );
        let now = Instant::now();
        let manual_stopwatch = self.widgets.render_manual_stopwatch(
            ui,
            self.state.snapshot(),
            &mut self.preferences,
            &self.manual_stopwatch_clock,
            now,
            WIDGET_MARGIN,
        );
        save_requested |= manual_stopwatch.save_requested;
        let client = &self.client;
        if widget_input_allowed {
            dispatch_manual_stopwatch_action(
                client,
                &mut self.manual_stopwatch_clock,
                self.state.snapshot().overlay_mode,
                manual_stopwatch.action,
                now,
            );
        }
        self.client
            .set_manual_stopwatch_running(self.manual_stopwatch_clock.running());
        let media = self.widgets.render_media(
            ui,
            self.state.snapshot(),
            &self.media_snapshot,
            &mut self.preferences,
            WIDGET_MARGIN,
        );
        save_requested |= media.save_requested;
        if widget_input_allowed
            && self.state.snapshot().overlay_mode == OverlayMode::Interactive
            && let Some(action) = media.action
        {
            let _ = self.media_client.send(&self.media_snapshot, action);
        }
        let notes = self.widgets.render_notes(
            ui,
            self.state.snapshot(),
            &mut self.notes_state,
            &mut self.preferences,
            WIDGET_MARGIN,
        );
        save_requested |= notes.save_requested;
        if widget_input_allowed {
            for command in notes.actions {
                dispatch_notes_action(
                    &self.notes_service,
                    &mut self.notes_state,
                    self.state.snapshot().overlay_mode,
                    command,
                );
            }
        }

        let twitch = self.widgets.render_twitch(
            ui,
            self.state.snapshot(),
            &mut self.twitch_state,
            &self.twitch_prefs,
            &mut self.preferences,
            now,
            WIDGET_MARGIN,
        );
        save_requested |= twitch.save_requested;
        if widget_input_allowed {
            for action in twitch.actions {
                self.dispatch_twitch_action(action);
            }
        }

        save_requested |= self.warframe.render(
            ui,
            &mut self.widgets,
            self.state.snapshot(),
            &mut self.preferences,
            WIDGET_MARGIN,
            widget_input_allowed,
        );

        if save_requested {
            self.commit_geometry_preferences(preferences_before_geometry);
        }

        if controls_visible(self.state.snapshot())
            && !self.widgets.catalog_open()
            && !self.about_open
        {
            let warframe_options_allowed = is_warframe_active(self.state.snapshot());
            let mut profile_actions = Vec::new();
            let mut twitch_actions = Vec::new();
            let mut warframe_actions = WarframeActionBatch::default();
            let mut actions = {
                let profile = &self.preferences;
                let twitch_prefs = &self.twitch_prefs;
                let warframe_prefs = self.warframe.prefs();
                let reward_catalog = self.warframe.reward_catalog();
                self.widgets
                    .paint_widget_controls(ui.ctx(), ui.max_rect(), profile, |id, ui| {
                        let has_options = matches!(
                            id,
                            WidgetId::Clock
                                | WidgetId::Performance
                                | WidgetId::Notes
                                | WidgetId::TwitchChat
                                | WidgetId::WarframeStatus
                                | WidgetId::WarframeFissures
                                | WidgetId::WarframeSortie
                                | WidgetId::WarframeInvasions
                        );
                        if has_options {
                            ui.separator();
                        }
                        paint_profile_options(ui, profile, id, &mut profile_actions);
                        match id {
                            WidgetId::TwitchChat => paint_twitch_options(
                                ui,
                                &mut self.twitch_state,
                                twitch_prefs,
                                &mut twitch_actions,
                            ),
                            WidgetId::WarframeStatus => paint_gated_options(
                                ui,
                                warframe_options_allowed,
                                "Available while Warframe is active.",
                                |ui| {
                                    paint_status_options(
                                        ui,
                                        warframe_prefs,
                                        &mut warframe_actions.status,
                                    );
                                },
                            ),
                            WidgetId::WarframeFissures => paint_gated_options(
                                ui,
                                warframe_options_allowed,
                                "Available while Warframe is active.",
                                |ui| {
                                    paint_fissure_options(
                                        ui,
                                        warframe_prefs,
                                        &mut warframe_actions.fissures,
                                    );
                                },
                            ),
                            WidgetId::WarframeSortie => paint_gated_options(
                                ui,
                                warframe_options_allowed,
                                "Available while Warframe is active.",
                                |ui| {
                                    paint_sortie_options(
                                        ui,
                                        warframe_prefs,
                                        &mut warframe_actions.sortie,
                                    );
                                },
                            ),
                            WidgetId::WarframeInvasions => paint_gated_options(
                                ui,
                                warframe_options_allowed,
                                "Available while Warframe is active.",
                                |ui| {
                                    paint_invasion_options(
                                        ui,
                                        warframe_prefs,
                                        reward_catalog,
                                        &mut warframe_actions.invasions,
                                    );
                                },
                            ),
                            _ => {}
                        }
                    })
            };
            actions.extend(profile_actions);
            for action in actions {
                self.apply_catalog_action(ui.ctx(), action);
            }
            for action in twitch_actions {
                self.dispatch_twitch_action(action);
            }
            self.warframe
                .apply_catalog_actions(self.state.snapshot(), warframe_actions);
        }

        if controls_visible(self.state.snapshot()) {
            paint_overlay_version(ui.ctx());
            let catalog_was_open = self.widgets.catalog_open();
            let mut toggle_catalog = false;
            let mut toggle_about = false;
            egui::Area::new(egui::Id::new("overlay-controls"))
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(PANEL_FILL)
                        .stroke(egui::Stroke::new(1.0, PANEL_STROKE_STRONG))
                        .corner_radius(14)
                        .inner_margin(egui::Margin::symmetric(14, 9))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                paint_brand(ui, &mut self.brand, BrandSize::Sm);
                                ui.separator();
                                shortcut_hint(ui, "SUPER + ALT + O", "TOGGLE");
                                shortcut_hint(ui, "ESC", "CLOSE");
                                ui.separator();
                                toggle_catalog = ui
                                    .add(
                                        egui::Button::new("Widget library")
                                            .selected(self.widgets.catalog_open()),
                                    )
                                    .clicked();
                                toggle_about = ui
                                    .add(egui::Button::new("About").selected(self.about_open))
                                    .clicked();
                            });
                            paint_control_notices(
                                ui,
                                self.warframe.message(),
                                self.widgets.catalog_message(),
                                self.twitch_state.message(),
                            );
                        });
                });

            if toggle_catalog {
                let open = !self.widgets.catalog_open();
                self.widgets.set_catalog_open(open);
                if open {
                    self.about_open = false;
                }
            }

            if toggle_about {
                self.about_open = !self.about_open;
                if self.about_open {
                    self.widgets.set_catalog_open(false);
                }
            }

            if catalog_visible(
                self.state.snapshot().overlay_mode,
                self.state.snapshot().active_game.is_some(),
                self.widgets.catalog_open(),
            ) {
                let (actions, catalog_rect) = paint_widget_catalog(
                    ui.ctx(),
                    ui.max_rect().size(),
                    &mut self.brand,
                    &self.preferences,
                    is_warframe_active(self.state.snapshot()),
                    self.widgets.catalog_message(),
                );

                for action in actions {
                    self.apply_catalog_action(ui.ctx(), action);
                }
                let pointer_click = ui.input(|input| {
                    input
                        .pointer
                        .any_click()
                        .then(|| input.pointer.interact_pos())
                        .flatten()
                });
                if catalog_outside_click(
                    catalog_was_open,
                    toggle_catalog,
                    pointer_click,
                    catalog_rect,
                ) {
                    self.widgets.set_catalog_open(false);
                }
            }

            if about_visible(self.state.snapshot(), self.about_open) {
                self.about_open =
                    paint_about_window(ui.ctx(), ui.max_rect().size(), &mut self.brand);
            }
        }

        self.widgets.sync_interaction_state(
            self.state.snapshot().overlay_mode,
            self.state.snapshot().active_game.is_some(),
            ui.input(|input| input.pointer.primary_down()),
        );
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

fn controls_visible(snapshot: &CoreSnapshot) -> bool {
    snapshot.active_game.is_some() && snapshot.overlay_mode == OverlayMode::Interactive
}

fn catalog_outside_click(
    was_open: bool,
    toggled_this_frame: bool,
    pointer_click: Option<egui::Pos2>,
    surface: egui::Rect,
) -> bool {
    was_open
        && !toggled_this_frame
        && pointer_click.is_some_and(|position| !surface.contains(position))
}

fn widget_actions_allowed(catalog_open: bool, about_open: bool) -> bool {
    !catalog_open && !about_open
}

fn about_content_size(viewport: egui::Vec2) -> egui::Vec2 {
    egui::vec2(
        (viewport.x - 96.0).clamp(180.0, 460.0),
        (viewport.y - 160.0).clamp(120.0, 520.0),
    )
}

fn paint_overlay_version(context: &egui::Context) {
    egui::Area::new(egui::Id::new("overlay-version"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -10.0))
        .interactable(false)
        .show(context, |ui| {
            ui.label(
                egui::RichText::new(APP_VERSION)
                    .monospace()
                    .size(9.0)
                    .color(TEXT_MUTED.gamma_multiply(0.62)),
            );
        });
}

fn paint_about_window(
    context: &egui::Context,
    viewport: egui::Vec2,
    brand: &mut BrandAssets,
) -> bool {
    let content_size = about_content_size(viewport);
    let frame = egui::Frame::window(&context.style_of(egui::Theme::Dark))
        .fill(PANEL_FILL)
        .stroke(egui::Stroke::new(1.0, PANEL_STROKE_STRONG))
        .corner_radius(16)
        .inner_margin(egui::Margin::same(22));
    let mut open = true;
    let mut close_requested = false;

    egui::Window::new("PlayerVox OverCrow")
        .id(egui::Id::new("overlay-about"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .title_bar(false)
        .resizable(false)
        .default_width(content_size.x)
        .frame(frame)
        .open(&mut open)
        .show(context, |ui| {
            ui.set_width(content_size.x);
            let header = ui.horizontal(|ui| {
                paint_brand(ui, brand, BrandSize::Md);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close_requested = about_close_button(ui).clicked();
                });
            });
            ui.add_space(14.0);
            let scroll_height = (content_size.y - header.response.rect.height() - 14.0).max(48.0);
            egui::ScrollArea::vertical()
                .max_height(scroll_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_width(content_size.x);
                    ui.label(
                        egui::RichText::new("Your games. Your overlay. Your control.")
                            .size(20.0)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "A lightweight external overlay that stays outside your game process.",
                        )
                        .color(TEXT_MUTED),
                    );
                    ui.add_space(16.0);
                    egui::Frame::new()
                        .fill(egui::Color32::from_white_alpha(8))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(22)))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::same(14))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("OPEN SOURCE")
                                    .size(10.0)
                                    .strong()
                                    .color(ACCENT),
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label("License");
                                ui.monospace(LICENSE_ID);
                            });
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(NOTICE_TEXT.trim())
                                    .size(11.0)
                                    .color(TEXT_MUTED),
                            );
                        });
                    ui.add_space(14.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.hyperlink_to("View source on GitHub", SOURCE_REPOSITORY_URL);
                        ui.separator();
                        ui.label(
                            egui::RichText::new("No warranty")
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );
                    });
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "PlayerVox trademark use is governed separately by TRADEMARKS.md.",
                        )
                        .size(11.0)
                        .color(TEXT_MUTED),
                    );
                    ui.add_space(18.0);
                    ui.label(
                        egui::RichText::new(format!("Version {APP_VERSION}"))
                            .monospace()
                            .size(10.0)
                            .color(TEXT_MUTED.gamma_multiply(0.72)),
                    );
                });
        });

    open && !close_requested
}

fn about_close_button(ui: &mut egui::Ui) -> egui::Response {
    let response = ui
        .add(egui::Button::new(egui::RichText::new("×").size(20.0).color(TEXT_MUTED)).frame(false));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Close")
    });
    response.on_hover_text("Close")
}

fn shortcut_hint(ui: &mut egui::Ui, key: &str, action: &str) {
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(egui::Color32::from_white_alpha(10))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(24)))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(7, 4))
            .show(ui, |ui| {
                ui.label(egui::RichText::new(key).monospace().strong().size(11.0));
            });
        ui.label(egui::RichText::new(action).size(10.0).color(TEXT_MUTED));
    });
}

fn paint_widget_catalog(
    context: &egui::Context,
    viewport: egui::Vec2,
    brand: &mut BrandAssets,
    profile: &overcrow_config::WidgetProfile,
    warframe_active: bool,
    message: Option<&str>,
) -> (Vec<CatalogAction>, egui::Rect) {
    let layout = CatalogLayout::for_viewport(viewport);
    let mut actions = Vec::new();
    let area = egui::Area::new(egui::Id::new("widget-catalog"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -82.0))
        .show(context, |ui| {
            egui::Frame::new()
                .fill(PANEL_FILL)
                .stroke(egui::Stroke::new(1.0, PANEL_STROKE_STRONG))
                .corner_radius(16)
                .inner_margin(egui::Margin::symmetric(18, 16))
                .show(ui, |ui| {
                    ui.set_width(layout.width);
                    ui.horizontal(|ui| {
                        paint_brand(ui, brand, BrandSize::Sm);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            status_pill(ui, "CUSTOMIZE MODE", ACCENT)
                        });
                    });
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical()
                        .max_height(layout.max_height)
                        .show(ui, |ui| {
                            ui.set_width(layout.width);
                            actions.extend(paint_catalog(ui, profile, message, warframe_active));
                        });
                });
        });
    (actions, area.response.rect)
}

fn about_visible(snapshot: &CoreSnapshot, about_open: bool) -> bool {
    about_open && controls_visible(snapshot)
}

fn interactive_scrim(snapshot: &CoreSnapshot) -> Option<egui::Color32> {
    (snapshot.active_game.is_some() && snapshot.overlay_mode == OverlayMode::Interactive)
        .then_some(egui::Color32::from_black_alpha(178))
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod app_tests;
