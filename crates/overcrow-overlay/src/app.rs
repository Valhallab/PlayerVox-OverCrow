use std::sync::Arc;

use crate::{
    branding::{BrandAssets, BrandSize, install_fonts, paint_brand},
    media::{MediaClient, MediaSnapshot},
    preferences::{OverlayPreferences, PreferenceStore},
    runtime::{ProviderReadiness, SnapshotClient, SnapshotUpdate},
    session_clock::SessionClock,
    warframe::WarframeController,
    widgets::{
        CatalogAction, CatalogActionOutcome, ManualStopwatchClock, WidgetManager,
        apply_catalog_action, catalog_visible, manual_stopwatch_repaint_after, paint_catalog,
        route_manual_stopwatch_action, session_repaint_after as stopwatch_repaint_after,
    },
};
use eframe::egui;
use overcrow_config::{WidgetId, settings_save_was_committed};
use overcrow_logging::EventLogger;
use overcrow_protocol::{CoreSnapshot, OverlayMode};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const APP_ID: &str = "io.github.overcrow.Overlay";
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
    fn from_snapshot(snapshot: &CoreSnapshot) -> Self {
        let (position, size) = snapshot
            .active_game
            .as_ref()
            .filter(|game| game.backend == "x11")
            .map_or((None, None), |game| {
                (
                    Some([game.rect.x as f32, game.rect.y as f32]),
                    Some([game.rect.width as f32, game.rect.height as f32]),
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

fn viewport_update_changed(previous: &CoreSnapshot, update: &ViewportUpdate) -> bool {
    ViewportUpdate::from_snapshot(previous) != *update
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

    fn apply_snapshot(&mut self, update: SnapshotUpdate) -> ViewportUpdate {
        if update.passive_confirmed {
            self.passive_pending = false;
        }
        if self.passive_pending {
            return ViewportUpdate::from_snapshot(&self.snapshot);
        }
        let viewport = ViewportUpdate::from_snapshot(&update.snapshot);
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
    warframe: WarframeController,
    preferences: OverlayPreferences,
    preference_store: PreferenceStore,
    widgets: WidgetManager,
    brand: BrandAssets,
    about_open: bool,
}

impl OverlayApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>, logger: EventLogger) -> Self {
        install_fonts(&creation_context.egui_ctx);
        let repaint_context = creation_context.egui_ctx.clone();
        let client_repaint_context = repaint_context.clone();
        let media_repaint_context = repaint_context.clone();
        let media_readiness = ProviderReadiness::default();
        let media_callback_readiness = media_readiness.clone();
        let client = SnapshotClient::spawn(logger.clone(), move || {
            client_repaint_context.request_repaint();
        });
        let media_client = MediaClient::spawn(logger.clone(), move || {
            media_callback_readiness.mark_media();
            media_repaint_context.request_repaint();
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
            warframe: WarframeController::new(&creation_context.egui_ctx, logger.clone()),
            preferences: preference_load.profile,
            preference_store,
            widgets: WidgetManager::default(),
            brand: BrandAssets::default(),
            about_open: false,
        }
    }

    fn apply_snapshot(&mut self, context: &egui::Context, snapshot: SnapshotUpdate) {
        let previous = self.state.snapshot().clone();
        let mode_event =
            confirmed_mode_event(previous.overlay_mode, self.state.passive_pending, &snapshot);
        let update = self.state.apply_snapshot(snapshot);
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
        if !viewport_update_changed(&previous, &update) {
            return;
        }

        context.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(
            update.mouse_passthrough,
        ));
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

    fn save_preferences(&self) {
        if let Err(error) = self.preference_store.save(&self.preferences) {
            eprintln!("OverCrow preference save failed: {error}");
            let category = if settings_save_was_committed(&error) {
                "durability"
            } else {
                "filesystem"
            };
            self.logger.warn(
                "widget_settings_save_failed",
                format_args!("{} category={category}", settings_failure_target(None)),
            );
        }
    }

    fn apply_catalog_action(&mut self, context: &egui::Context, action: CatalogAction) {
        let widget_id = action.widget_id();
        let outcome = apply_catalog_action(&mut self.preferences, action, |candidate| {
            self.preference_store.save(candidate)
        });
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
        None => "affected_widgets=layout",
    }
}

fn log_catalog_settings_outcome(
    logger: &EventLogger,
    widget_id: WidgetId,
    outcome: &CatalogActionOutcome,
) {
    let category = match outcome {
        CatalogActionOutcome::Durable(_) => return,
        CatalogActionOutcome::CommittedWithWarning { .. } => "durability",
        CatalogActionOutcome::RolledBack { category, .. } => category.name(),
    };
    logger.warn(
        "widget_settings_save_failed",
        format_args!(
            "{} category={category}",
            settings_failure_target(Some(widget_id))
        ),
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

impl eframe::App for OverlayApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let wall_secs = wall_secs();
        if let Some(snapshot) = self.client.take_latest() {
            self.apply_snapshot(context, snapshot);
        }
        if self.media_readiness.take().media()
            && let Some(snapshot) = self.media_client.take_latest()
        {
            self.media_revision = snapshot.revision;
            self.media_snapshot = snapshot.value;
        }
        self.warframe.sync(
            context,
            self.state.snapshot(),
            &self.preferences,
            now,
            wall_secs,
        );
        if context.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.request_passive(context);
        }

        let next_repaint = [
            stopwatch_repaint_after(
                self.state.snapshot(),
                &self.preferences,
                &self.session_clock,
                now,
            ),
            manual_stopwatch_repaint_after(
                self.state.snapshot(),
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
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
        dispatch_manual_stopwatch_action(
            client,
            &mut self.manual_stopwatch_clock,
            self.state.snapshot().overlay_mode,
            manual_stopwatch.action,
            now,
        );
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
        if self.state.snapshot().overlay_mode == OverlayMode::Interactive
            && let Some(action) = media.action
        {
            let _ = self.media_client.send(&self.media_snapshot, action);
        }

        save_requested |= self.warframe.render(
            ui,
            &mut self.widgets,
            self.state.snapshot(),
            &mut self.preferences,
            WIDGET_MARGIN,
        );

        if save_requested {
            self.save_preferences();
        }

        if controls_visible(self.state.snapshot()) {
            let mut toggle_catalog = false;
            let mut toggle_about = false;
            egui::Area::new(egui::Id::new("overlay-controls"))
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_black_alpha(220))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(36)))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    paint_brand(ui, &mut self.brand, BrandSize::Sm);
                                    ui.separator();
                                    ui.label(
                                        egui::RichText::new("Super + Alt + O").monospace().strong(),
                                    );
                                    ui.label("open/close");
                                    ui.separator();
                                    ui.label(egui::RichText::new("Esc").monospace().strong());
                                    ui.label("close");
                                    ui.separator();
                                    toggle_catalog = ui.button("Widgets").clicked();
                                    toggle_about = ui.button("About").clicked();
                                });
                                if let Some(message) = self.warframe.message() {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(255, 150, 150),
                                        message,
                                    );
                                }
                            });
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
                let mut actions = Vec::new();
                egui::Area::new(egui::Id::new("widget-catalog"))
                    .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -82.0))
                    .show(ui.ctx(), |ui| {
                        egui::Frame::new()
                            .fill(egui::Color32::from_black_alpha(230))
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(36)))
                            .corner_radius(10)
                            .inner_margin(egui::Margin::symmetric(14, 12))
                            .show(ui, |ui| {
                                paint_brand(ui, &mut self.brand, BrandSize::Md);
                                ui.add_space(6.0);
                                egui::ScrollArea::vertical()
                                    .max_height(420.0)
                                    .show(ui, |ui| {
                                        actions.extend(paint_catalog(
                                            ui,
                                            &self.preferences,
                                            self.widgets.catalog_message(),
                                        ));
                                    });
                            });
                    });

                for action in actions {
                    self.apply_catalog_action(ui.ctx(), action);
                }
            }

            if about_visible(self.state.snapshot(), self.about_open) {
                let mut open = self.about_open;
                egui::Window::new("About OverCrow")
                    .id(egui::Id::new("overlay-about"))
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .collapsible(false)
                    .resizable(false)
                    .default_width(420.0)
                    .open(&mut open)
                    .show(ui.ctx(), |ui| {
                        paint_brand(ui, &mut self.brand, BrandSize::Md);
                        ui.separator();
                        ui.label(NOTICE_TEXT.trim());
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label("License:");
                            ui.monospace(LICENSE_ID);
                        });
                        ui.weak(
                            "This software is provided without warranty; see LICENSE for details.",
                        );
                        ui.add_space(6.0);
                        ui.hyperlink_to("Source code", SOURCE_REPOSITORY_URL);
                        ui.weak("PlayerVox trademark use is governed separately by TRADEMARKS.md.");
                    });
                self.about_open = open;
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
