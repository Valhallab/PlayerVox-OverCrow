use std::{
    io,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use eframe::egui;
use overcrow_config::{
    WARFRAME_MARKET_QUERY_MAX_CHARS, WarframePrefs, WarframePrefsStore, WidgetProfile,
    settings_save_was_committed,
};
use overcrow_logging::EventLogger;
use overcrow_protocol::{CoreSnapshot, OverlayMode};

use super::{
    MarketClient, MarketCommand, MarketSnapshot, WarframeDerivedCache, WorldstateClient,
    WorldstateSnapshot, advance_live_timers, any_worldstate_widget_enabled, copy_to_clipboard,
    current_activity_done_keys, is_warframe_active, market_requests_enabled,
};
use crate::{
    runtime::{OverlayScheduler, ProviderReadiness},
    widgets::{
        FissurePrefsAction, InvasionPrefsAction, MarketUiAction, SortiePrefsAction,
        StatusPrefsAction, WidgetManager, apply_fissure_prefs_action, apply_invasion_prefs_action,
        apply_sortie_prefs_action, apply_status_prefs_action,
    },
};

pub(super) const WARFRAME_PREFS_ERROR_MAX_CHARS: usize = 180;
const COPY_FLASH_DURATION: Duration = Duration::from_millis(1_400);

#[derive(Default)]
pub(super) struct WarframeActionBatch {
    pub(super) status: Vec<StatusPrefsAction>,
    pub(super) fissures: Vec<FissurePrefsAction>,
    pub(super) sortie: Vec<SortiePrefsAction>,
    pub(super) invasions: Vec<InvasionPrefsAction>,
    pub(super) market_query: Option<String>,
}

impl WarframeActionBatch {
    fn is_empty(&self) -> bool {
        self.status.is_empty()
            && self.fissures.is_empty()
            && self.sortie.is_empty()
            && self.invasions.is_empty()
            && self.market_query.is_none()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WarframePrefsCommitOutcome {
    Durable,
    CommittedWithWarning {
        message: String,
    },
    RolledBack {
        message: String,
        category: WarframePrefsCommitFailure,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WarframePrefsCommitFailure {
    Validation,
    Filesystem,
}

impl WarframePrefsCommitFailure {
    const fn name(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Filesystem => "filesystem",
        }
    }
}

pub struct WarframeController {
    logger: EventLogger,
    pub(super) worldstate_client: WorldstateClient,
    pub(super) worldstate_snapshot: Arc<WorldstateSnapshot>,
    pub(super) worldstate_revision: u64,
    pub(super) readiness: ProviderReadiness,
    pub(super) scheduler: OverlayScheduler,
    pub(super) derived: WarframeDerivedCache,
    pub(super) market_client: MarketClient,
    pub(super) market_snapshot: Arc<MarketSnapshot>,
    pub(super) market_revision: u64,
    pub(super) market_query: String,
    pub(super) market_copy_flash: Option<(String, Instant)>,
    pub(super) prefs: WarframePrefs,
    pub(super) prefs_revision: u64,
    pub(super) prefs_store: WarframePrefsStore,
    pub(super) prefs_message: Option<String>,
    pub(super) worldstate_enabled: bool,
    pub(super) market_enabled: bool,
}

pub(super) struct WarframeProviderUpdates {
    pub(super) worldstate: Option<crate::runtime::VersionedValue<WorldstateSnapshot>>,
    pub(super) market: Option<crate::runtime::VersionedValue<MarketSnapshot>>,
}

pub(super) fn drain_ready_warframe_providers<W, M>(
    readiness: &ProviderReadiness,
    take_worldstate: W,
    take_market: M,
) -> WarframeProviderUpdates
where
    W: FnOnce() -> Option<crate::runtime::VersionedValue<WorldstateSnapshot>>,
    M: FnOnce() -> Option<crate::runtime::VersionedValue<MarketSnapshot>>,
{
    let ready: crate::runtime::ReadyProviders = readiness.take();
    WarframeProviderUpdates {
        worldstate: if ready.worldstate() {
            take_worldstate()
        } else {
            None
        },
        market: if ready.market() { take_market() } else { None },
    }
}

impl WarframeController {
    pub fn new(context: &egui::Context, logger: EventLogger) -> Self {
        let readiness = ProviderReadiness::default();
        let worldstate_readiness = readiness.clone();
        let market_readiness = readiness.clone();
        let worldstate_context = context.clone();
        let market_context = context.clone();
        let store = WarframePrefsStore::from_environment();
        let load = store.load();
        if let Some(warning) = &load.warning {
            eprintln!("OverCrow Warframe prefs rejected; using defaults: {warning}");
            logger.warn(
                "widget_settings_load_failed",
                format_args!(
                    "affected_widgets=warframe_status,warframe_fissures,warframe_market,warframe_sortie,warframe_invasions category=validation"
                ),
            );
        }

        Self::with_dependencies(
            WorldstateClient::new(logger.clone(), move || {
                worldstate_readiness.mark_worldstate();
                worldstate_context.request_repaint();
            }),
            MarketClient::new(logger.clone(), move || {
                market_readiness.mark_market();
                market_context.request_repaint();
            }),
            store,
            load.prefs,
            readiness,
            logger,
        )
    }

    pub(super) fn with_dependencies(
        worldstate_client: WorldstateClient,
        market_client: MarketClient,
        prefs_store: WarframePrefsStore,
        prefs: WarframePrefs,
        readiness: ProviderReadiness,
        logger: EventLogger,
    ) -> Self {
        let market_query = prefs.last_market_query.clone();
        Self {
            logger,
            worldstate_client,
            worldstate_snapshot: Arc::new(WorldstateSnapshot::default()),
            worldstate_revision: 0,
            readiness,
            scheduler: OverlayScheduler::default(),
            derived: WarframeDerivedCache::default(),
            market_client,
            market_snapshot: Arc::new(MarketSnapshot::default()),
            market_revision: 0,
            market_query,
            market_copy_flash: None,
            prefs,
            prefs_revision: 0,
            prefs_store,
            prefs_message: None,
            worldstate_enabled: false,
            market_enabled: false,
        }
    }

    pub fn sync(
        &mut self,
        context: &egui::Context,
        snapshot: &CoreSnapshot,
        profile: &WidgetProfile,
        now: Instant,
        wall_secs: u64,
    ) {
        self.worldstate_enabled =
            is_warframe_active(snapshot) && any_worldstate_widget_enabled(profile);
        self.market_enabled = market_requests_enabled(snapshot, profile);
        self.worldstate_client
            .set_polling_enabled(self.worldstate_enabled);
        self.market_client.set_enabled(self.market_enabled);

        let updates = drain_ready_warframe_providers(
            &self.readiness,
            || self.worldstate_client.take_latest(),
            || self.market_client.take_latest(),
        );
        if let Some(update) = updates.worldstate {
            self.worldstate_revision = update.revision;
            self.worldstate_snapshot = update.value;
        }
        if let Some(update) = updates.market {
            self.market_revision = update.revision;
            self.market_snapshot = update.value;
        }

        if self
            .scheduler
            .take_warframe_tick(self.worldstate_enabled, now)
            && advance_live_timers(Arc::make_mut(&mut self.worldstate_snapshot), wall_secs)
        {
            self.worldstate_client.request_refresh();
        }
        self.derived.sync(
            &self.worldstate_snapshot,
            self.worldstate_revision,
            &self.prefs,
            self.prefs_revision,
        );

        if self.scheduler.take_market_refresh(
            self.market_enabled,
            self.market_snapshot.selected.is_some(),
            self.market_revision,
            self.market_snapshot.next_refresh_at_secs,
            now,
            wall_secs,
        ) {
            self.market_client.send(MarketCommand::RefreshSelected);
        }

        if self.scheduler.take_copy_flash_expired(now) {
            self.market_copy_flash = None;
        }
        if let Some(delay) = self.scheduler.next_repaint_after(now) {
            context.request_repaint_after(delay);
        }
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        widgets: &mut WidgetManager,
        snapshot: &CoreSnapshot,
        profile: &mut WidgetProfile,
        margin: f32,
    ) -> bool {
        let wall_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let allow_actions = warframe_actions_allowed(snapshot);
        let status = widgets.render_warframe_status(
            ui,
            snapshot,
            &self.worldstate_snapshot,
            &self.prefs,
            profile,
            wall_secs,
            margin,
        );
        let fissure_indices = Arc::clone(self.derived.fissure_indices());
        let invasion_indices = Arc::clone(self.derived.invasion_indices());
        let reward_catalog = Arc::clone(self.derived.reward_catalog());
        let fissures = widgets.render_warframe_fissures(
            ui,
            snapshot,
            &self.worldstate_snapshot,
            &fissure_indices,
            &self.prefs,
            profile,
            wall_secs,
            margin,
        );
        let sortie = widgets.render_warframe_sortie(
            ui,
            snapshot,
            &self.worldstate_snapshot,
            &self.prefs,
            profile,
            wall_secs,
            margin,
        );
        let invasions = widgets.render_warframe_invasions(
            ui,
            snapshot,
            &self.worldstate_snapshot,
            &invasion_indices,
            &reward_catalog,
            &mut self.derived,
            self.worldstate_revision,
            self.prefs_revision,
            &self.prefs,
            profile,
            margin,
        );
        let copy_flash_id = self
            .market_copy_flash
            .as_ref()
            .filter(|(_, until)| Instant::now() < *until)
            .map(|(id, _)| id.as_str());
        let market = widgets.render_warframe_market(
            ui,
            snapshot,
            &self.market_snapshot,
            &mut self.market_query,
            copy_flash_id,
            profile,
            margin,
        );

        let save_requested = status.save_requested
            || fissures.save_requested
            || sortie.save_requested
            || invasions.save_requested
            || market.save_requested;
        let mut batch = WarframeActionBatch::default();
        if allow_actions {
            batch.status = status.actions;
            batch.fissures = fissures.actions;
            batch.sortie = sortie.actions;
            batch.invasions = invasions.actions;
            for action in market.actions {
                match action {
                    MarketUiAction::Command(command) => {
                        let command = match command {
                            MarketCommand::Search(query) => {
                                let query = bounded_market_query(&query);
                                self.market_query = query.clone();
                                batch.market_query = Some(query.clone());
                                MarketCommand::Search(query)
                            }
                            command => command,
                        };
                        self.market_client.send(command);
                    }
                    MarketUiAction::CopyTrade { text, flash_id } => {
                        let copied = copy_to_clipboard(ui.ctx(), &text).is_ok();
                        let now = Instant::now();
                        self.set_copy_flash(flash_id, now);
                        if copied {
                            ui.ctx().request_repaint_after(COPY_FLASH_DURATION);
                        }
                    }
                }
            }
        }

        if !batch.is_empty() {
            let resource_keys = invasion_resource_keys(&batch, &self.derived);
            let store = &self.prefs_store;
            if let Some(outcome) = apply_action_batch(
                &mut self.prefs,
                &mut self.prefs_revision,
                &self.worldstate_snapshot,
                resource_keys,
                batch,
                |candidate| store.save(candidate),
            ) {
                log_warframe_prefs_outcome(&self.logger, &outcome);
                self.prefs_message = outcome_message(outcome);
            }
        }

        // Only global WidgetProfile changes escape the controller.
        save_requested
    }

    pub fn message(&self) -> Option<&str> {
        self.prefs_message.as_deref()
    }

    pub(super) fn set_copy_flash(&mut self, flash_id: String, now: Instant) {
        let deadline = now.checked_add(COPY_FLASH_DURATION).unwrap_or(now);
        self.market_copy_flash = Some((flash_id, deadline));
        self.scheduler.set_copy_flash_deadline(Some(deadline));
    }

    #[cfg(test)]
    pub(super) fn apply_action_batch_with_save<S>(
        &mut self,
        snapshot: &CoreSnapshot,
        batch: WarframeActionBatch,
        save: S,
    ) where
        S: FnOnce(&WarframePrefs) -> io::Result<()>,
    {
        if !warframe_actions_allowed(snapshot) || batch.is_empty() {
            return;
        }
        let resource_keys = invasion_resource_keys(&batch, &self.derived);
        if let Some(outcome) = apply_action_batch(
            &mut self.prefs,
            &mut self.prefs_revision,
            &self.worldstate_snapshot,
            resource_keys,
            batch,
            save,
        ) {
            log_warframe_prefs_outcome(&self.logger, &outcome);
            self.prefs_message = outcome_message(outcome);
        }
    }
}

pub(super) fn bounded_market_query(query: &str) -> String {
    query
        .trim()
        .chars()
        .take(WARFRAME_MARKET_QUERY_MAX_CHARS)
        .collect()
}

pub(super) fn warframe_actions_allowed(snapshot: &CoreSnapshot) -> bool {
    snapshot.overlay_mode == OverlayMode::Interactive && is_warframe_active(snapshot)
}

pub(super) fn prune_activity_done_for_snapshot(
    prefs: &mut WarframePrefs,
    snapshot: &WorldstateSnapshot,
) {
    prefs.prune_activity_done(&current_activity_done_keys(snapshot));
}

fn invasion_resource_keys(
    batch: &WarframeActionBatch,
    derived: &WarframeDerivedCache,
) -> Vec<String> {
    if batch
        .invasions
        .iter()
        .any(|action| matches!(action, InvasionPrefsAction::ToggleResourceFilter(_)))
    {
        derived
            .reward_catalog()
            .iter()
            .map(|(key, _)| key.clone())
            .collect()
    } else {
        Vec::new()
    }
}

fn apply_action_batch<S>(
    prefs: &mut WarframePrefs,
    prefs_revision: &mut u64,
    worldstate: &WorldstateSnapshot,
    resource_keys: Vec<String>,
    batch: WarframeActionBatch,
    save: S,
) -> Option<WarframePrefsCommitOutcome>
where
    S: FnOnce(&WarframePrefs) -> io::Result<()>,
{
    commit_warframe_prefs_if_dirty(
        prefs,
        prefs_revision,
        !batch.is_empty(),
        WarframePrefs::clone,
        |candidate| {
            for action in batch.status {
                apply_status_prefs_action(candidate, action);
            }
            for action in batch.fissures {
                apply_fissure_prefs_action(candidate, action);
            }
            for action in batch.sortie {
                if matches!(
                    &action,
                    SortiePrefsAction::ToggleDone(_) | SortiePrefsAction::SetBlockDone { .. }
                ) {
                    prune_activity_done_for_snapshot(candidate, worldstate);
                }
                apply_sortie_prefs_action(candidate, action);
            }
            for action in batch.invasions {
                if matches!(&action, InvasionPrefsAction::ToggleDone(_)) {
                    prune_activity_done_for_snapshot(candidate, worldstate);
                }
                apply_invasion_prefs_action(candidate, action, &resource_keys);
            }
            if let Some(query) = batch.market_query {
                candidate.last_market_query = bounded_market_query(&query);
            }
        },
        save,
    )
}

fn outcome_message(outcome: WarframePrefsCommitOutcome) -> Option<String> {
    match outcome {
        WarframePrefsCommitOutcome::Durable => None,
        WarframePrefsCommitOutcome::CommittedWithWarning { message }
        | WarframePrefsCommitOutcome::RolledBack { message, .. } => Some(message),
    }
}

pub(super) fn log_warframe_prefs_outcome(
    logger: &EventLogger,
    outcome: &WarframePrefsCommitOutcome,
) {
    const TARGET: &str = "affected_widgets=warframe_status,warframe_fissures,warframe_market,warframe_sortie,warframe_invasions";

    match outcome {
        WarframePrefsCommitOutcome::Durable => {}
        WarframePrefsCommitOutcome::CommittedWithWarning { .. } => logger.warn(
            "widget_settings_save_failed",
            format_args!("{TARGET} category=durability"),
        ),
        WarframePrefsCommitOutcome::RolledBack { category, .. } => logger.warn(
            "widget_settings_save_failed",
            format_args!("{TARGET} category={}", category.name()),
        ),
    }
}

pub(super) fn commit_warframe_prefs<S>(
    current: &mut WarframePrefs,
    candidate: WarframePrefs,
    save: S,
) -> WarframePrefsCommitOutcome
where
    S: FnOnce(&WarframePrefs) -> io::Result<()>,
{
    let candidate = match candidate.validate() {
        Ok(candidate) => candidate,
        Err(error) => {
            return WarframePrefsCommitOutcome::RolledBack {
                message: bounded_warframe_prefs_error(error),
                category: WarframePrefsCommitFailure::Validation,
            };
        }
    };

    match save(&candidate) {
        Ok(()) => {
            *current = candidate;
            WarframePrefsCommitOutcome::Durable
        }
        Err(error) if settings_save_was_committed(&error) => {
            *current = candidate;
            WarframePrefsCommitOutcome::CommittedWithWarning {
                message: bounded_warframe_prefs_durability_warning(error),
            }
        }
        Err(error) => WarframePrefsCommitOutcome::RolledBack {
            message: bounded_warframe_prefs_error(error),
            category: WarframePrefsCommitFailure::Filesystem,
        },
    }
}

pub(super) fn commit_warframe_prefs_if_dirty<S, C, A>(
    current: &mut WarframePrefs,
    revision: &mut u64,
    dirty: bool,
    clone_current: C,
    apply_actions: A,
    save: S,
) -> Option<WarframePrefsCommitOutcome>
where
    S: FnOnce(&WarframePrefs) -> io::Result<()>,
    C: FnOnce(&WarframePrefs) -> WarframePrefs,
    A: FnOnce(&mut WarframePrefs),
{
    if !dirty {
        return None;
    }

    let mut candidate = clone_current(current);
    apply_actions(&mut candidate);
    let outcome = commit_warframe_prefs(current, candidate, save);
    if matches!(
        outcome,
        WarframePrefsCommitOutcome::Durable
            | WarframePrefsCommitOutcome::CommittedWithWarning { .. }
    ) {
        *revision = revision.wrapping_add(1);
    }
    Some(outcome)
}

pub(super) fn bounded_warframe_prefs_error(error: impl std::fmt::Display) -> String {
    bounded_warframe_prefs_message("Could not apply Warframe preferences", error)
}

fn bounded_warframe_prefs_durability_warning(error: impl std::fmt::Display) -> String {
    bounded_warframe_prefs_message(
        "Saved Warframe preferences, but durability is uncertain",
        error,
    )
}

fn bounded_warframe_prefs_message(prefix: &str, error: impl std::fmt::Display) -> String {
    let message = format!("{prefix}: {error}");
    if message.chars().count() <= WARFRAME_PREFS_ERROR_MAX_CHARS {
        return message;
    }

    let mut bounded = message
        .chars()
        .take(WARFRAME_PREFS_ERROR_MAX_CHARS.saturating_sub(1))
        .collect::<String>();
    bounded.push('…');
    bounded
}
