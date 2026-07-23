use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use overcrow_logging::EventLogger;
use serde::Deserialize;

use super::model::{
    MARKET_ORDERS_REFRESH_SECS, MARKET_ORDERS_SHOWN, MARKET_RESULTS_MAX, MarketItemDetail,
    MarketItemSummary, MarketOrder, MarketSnapshot, TradeSide, TraderPresence, bound_error,
};
use crate::{
    runtime::{
        LatestPublisher, LatestReceiver, VersionedValue, latest_channel,
        widget_diagnostics::{FailureCategory, Provider, ProviderDiagnostics},
    },
    warframe::{
        http::{
            HttpError, MARKET_HOST, MARKET_MAX_BYTES, http_failure_category, https_get_allowlisted,
            is_safe_market_slug,
        },
        model::{STRING_MAX_CHARS, bound_chars},
        sanitize::{sanitize_item_name, sanitize_player_name},
    },
};

const ITEMS_URL: &str = "https://api.warframe.market/v2/items";
const ORDERS_URL_PREFIX: &str = "https://api.warframe.market/v2/orders/item/";
const COMMAND_CAPACITY: usize = 8;
const CATALOG_ITEM_MAX: usize = 50_000;
const ERROR_BACKOFF_INITIAL: Duration = Duration::from_secs(5);
const ERROR_BACKOFF_MAX: Duration = Duration::from_secs(120);
const RETRY_AFTER_MAX: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarketCommand {
    Search(String),
    Select(String),
    /// Re-fetch orders for the currently selected slug (silent if none).
    RefreshSelected,
    Clear,
}

struct WorkerCommand {
    generation: u64,
    command: MarketCommand,
}

struct WorkerControls {
    enabled: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    refresh_pending: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    publication_gate: Arc<Mutex<()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RetryPolicy {
    pub delay: Duration,
    pub next_backoff: Duration,
}

pub(super) fn retry_policy(error: &HttpError, backoff: Duration) -> RetryPolicy {
    if let HttpError::Status {
        code: 429,
        retry_after: Some(retry_after),
    } = error
    {
        return RetryPolicy {
            delay: (*retry_after).min(RETRY_AFTER_MAX),
            next_backoff: backoff,
        };
    }
    RetryPolicy {
        delay: backoff,
        next_backoff: (backoff * 2).min(ERROR_BACKOFF_MAX),
    }
}

pub(super) fn normalize_command(command: MarketCommand) -> Option<MarketCommand> {
    Some(match command {
        MarketCommand::Search(query) => {
            MarketCommand::Search(bound_chars(query.trim(), STRING_MAX_CHARS))
        }
        MarketCommand::Select(slug) if is_safe_market_slug(&slug) => MarketCommand::Select(slug),
        MarketCommand::Select(_) => return None,
        command => command,
    })
}

pub struct MarketClient {
    publisher: LatestPublisher<MarketSnapshot>,
    latest: LatestReceiver<MarketSnapshot>,
    commands: Option<SyncSender<WorkerCommand>>,
    enabled: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    refresh_pending: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    publication_gate: Arc<Mutex<()>>,
    request_repaint: Arc<dyn Fn() + Send + Sync>,
    join: Option<JoinHandle<()>>,
}

impl Default for MarketClient {
    fn default() -> Self {
        Self::new(EventLogger::disabled(), || {})
    }
}

impl MarketClient {
    pub fn new(logger: EventLogger, request_repaint: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_backend_clock_and_repaint(
            LiveMarketBackend::default(),
            now_secs,
            logger,
            request_repaint,
        )
    }

    #[cfg(test)]
    pub fn with_backend<B>(backend: B) -> Self
    where
        B: MarketBackend + Send + 'static,
    {
        Self::with_backend_clock_and_repaint(backend, now_secs, EventLogger::disabled(), || {})
    }

    #[cfg(test)]
    pub(super) fn with_backend_and_logger<B>(backend: B, logger: EventLogger) -> Self
    where
        B: MarketBackend + Send + 'static,
    {
        Self::with_backend_clock_and_repaint(backend, now_secs, logger, || {})
    }

    #[cfg(test)]
    pub(super) fn with_backend_and_clock<B, C>(backend: B, clock: C) -> Self
    where
        B: MarketBackend + Send + 'static,
        C: Fn() -> u64 + Send + Sync + 'static,
    {
        Self::with_backend_clock_and_repaint(backend, clock, EventLogger::disabled(), || {})
    }

    fn with_backend_clock_and_repaint<B, C, R>(
        backend: B,
        clock: C,
        logger: EventLogger,
        request_repaint: R,
    ) -> Self
    where
        B: MarketBackend + Send + 'static,
        C: Fn() -> u64 + Send + Sync + 'static,
        R: Fn() + Send + Sync + 'static,
    {
        let (publisher, latest) = latest_channel(MarketSnapshot::default());
        let (tx, rx) = mpsc::sync_channel(COMMAND_CAPACITY);
        let enabled = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let refresh_pending = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));
        let publication_gate = Arc::new(Mutex::new(()));
        let request_repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(request_repaint);
        let worker_controls = WorkerControls {
            enabled: Arc::clone(&enabled),
            generation: Arc::clone(&generation),
            refresh_pending: Arc::clone(&refresh_pending),
            shutdown: Arc::clone(&shutdown),
            publication_gate: Arc::clone(&publication_gate),
        };
        let worker_publisher = publisher.clone();
        let worker_repaint = Arc::clone(&request_repaint);
        let join = thread::Builder::new()
            .name("overcrow-warframe-market".to_owned())
            .spawn(move || {
                worker_loop(
                    worker_publisher,
                    rx,
                    backend,
                    worker_controls,
                    ProviderDiagnostics::new(logger, Provider::WarframeMarket),
                    clock,
                    move || worker_repaint(),
                );
            })
            .expect("spawn market worker");
        Self {
            publisher,
            latest,
            commands: Some(tx),
            enabled,
            generation,
            refresh_pending,
            shutdown,
            publication_gate,
            request_repaint,
            join: Some(join),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        let _publication = self
            .publication_gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let previous = self.enabled.swap(enabled, Ordering::SeqCst);
        if previous == enabled {
            return;
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        if !enabled {
            let current = self.publisher.current();
            if (current.value.status.is_some() || current.value.next_refresh_at_secs != 0)
                && self.publisher.update(|snapshot| {
                    snapshot.status = None;
                    snapshot.next_refresh_at_secs = 0;
                })
            {
                (self.request_repaint)();
            }
        }
    }

    pub fn take_latest(&self) -> Option<VersionedValue<MarketSnapshot>> {
        self.latest.take_latest()
    }

    #[cfg(test)]
    pub fn latest(&self) -> MarketSnapshot {
        self.publisher.current().value.as_ref().clone()
    }

    pub fn send(&self, command: MarketCommand) {
        let Some(command) = normalize_command(command) else {
            return;
        };
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }
        let Some(commands) = &self.commands else {
            return;
        };
        let is_refresh = command == MarketCommand::RefreshSelected;
        if is_refresh && self.refresh_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let request = WorkerCommand {
            generation: self.generation.load(Ordering::SeqCst),
            command,
        };
        match commands.try_send(request) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                if is_refresh {
                    self.refresh_pending.store(false, Ordering::SeqCst);
                }
                if self.publisher.current().value.status.as_deref() != Some("Market busy…")
                    && self.publisher.update(|snapshot| {
                        snapshot.status = Some("Market busy…".to_owned());
                    })
                {
                    (self.request_repaint)();
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                if is_refresh {
                    self.refresh_pending.store(false, Ordering::SeqCst);
                }
                if self.publisher.current().value.error.as_deref()
                    != Some("Market client unavailable")
                    && self.publisher.update(|snapshot| {
                        snapshot.error = Some("Market client unavailable".to_owned());
                    })
                {
                    (self.request_repaint)();
                }
            }
        }
    }
}

impl Drop for MarketClient {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.enabled.store(false, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.commands.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub trait MarketBackend {
    fn search(&mut self, query: &str) -> Result<Vec<MarketItemSummary>, HttpError>;
    fn orders(&mut self, slug: &str) -> Result<MarketItemDetail, HttpError>;
}

#[derive(Default)]
struct LiveMarketBackend {
    catalog: Option<Vec<CatalogItem>>,
}

#[derive(Clone, Debug)]
struct CatalogItem {
    name: String,
    slug: String,
}

impl MarketBackend for LiveMarketBackend {
    fn search(&mut self, query: &str) -> Result<Vec<MarketItemSummary>, HttpError> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        if needle.chars().count() > STRING_MAX_CHARS {
            return Err(HttpError::Read("request too long".to_owned()));
        }
        let needle = needle.to_ascii_lowercase();
        let catalog = self.ensure_catalog()?;
        Ok(catalog
            .iter()
            .filter(|item| item.name.to_ascii_lowercase().contains(&needle))
            .take(MARKET_RESULTS_MAX)
            .map(|item| MarketItemSummary {
                name: item.name.clone(),
                slug: item.slug.clone(),
            })
            .collect())
    }

    fn orders(&mut self, slug: &str) -> Result<MarketItemDetail, HttpError> {
        if !is_safe_market_slug(slug) {
            return Err(HttpError::InvalidUrl);
        }
        let url = format!("{ORDERS_URL_PREFIX}{slug}");
        let body = https_get_allowlisted(&url, MARKET_HOST, MARKET_MAX_BYTES)?;
        let parsed: OrdersResponse = serde_json::from_slice(&body)
            .map_err(|error| HttpError::Parse(bound_error(&error.to_string())))?;
        let mut lowest_sell: Option<u32> = None;
        let mut highest_buy: Option<u32> = None;
        let mut order_count = 0u32;
        let mut sells = Vec::new();
        let mut buys = Vec::new();

        for order in parsed.data {
            if !order.visible || order.platinum == 0 || order.platinum > 1_000_000 {
                continue;
            }
            let trader = order
                .user
                .as_ref()
                .and_then(|user| user.ingame_name.as_deref())
                .map(sanitize_player_name)
                .filter(|name| !name.is_empty());
            let Some(trader) = trader else {
                continue;
            };
            let presence = order
                .user
                .as_ref()
                .and_then(|user| user.status.as_deref())
                .map(parse_presence)
                .unwrap_or(TraderPresence::Unknown);

            order_count = order_count.saturating_add(1);
            match order.order_type.as_str() {
                "sell" => {
                    lowest_sell = Some(match lowest_sell {
                        Some(current) => current.min(order.platinum),
                        None => order.platinum,
                    });
                    sells.push(MarketOrder {
                        side: TradeSide::Sell,
                        platinum: order.platinum,
                        trader,
                        presence,
                    });
                }
                "buy" => {
                    highest_buy = Some(match highest_buy {
                        Some(current) => current.max(order.platinum),
                        None => order.platinum,
                    });
                    buys.push(MarketOrder {
                        side: TradeSide::Buy,
                        platinum: order.platinum,
                        trader,
                        presence,
                    });
                }
                _ => {}
            }
        }

        // Prefer players who can trade now, then best price for the viewer.
        sells.sort_by_key(|order| (order.presence.rank(), order.platinum, order.trader.clone()));
        buys.sort_by_key(|order| {
            (
                order.presence.rank(),
                std::cmp::Reverse(order.platinum),
                order.trader.clone(),
            )
        });
        sells.truncate(MARKET_ORDERS_SHOWN);
        buys.truncate(MARKET_ORDERS_SHOWN);

        let name = self
            .ensure_catalog()?
            .iter()
            .find(|item| item.slug == slug)
            .map(|item| item.name.clone())
            .unwrap_or_else(|| sanitize_item_name(slug));
        Ok(MarketItemDetail {
            name,
            slug: bound_chars(slug, STRING_MAX_CHARS),
            lowest_sell,
            highest_buy,
            order_count,
            top_sells: sells,
            top_buys: buys,
        })
    }
}

impl LiveMarketBackend {
    fn ensure_catalog(&mut self) -> Result<&[CatalogItem], HttpError> {
        if self.catalog.is_none() {
            let body = https_get_allowlisted(ITEMS_URL, MARKET_HOST, MARKET_MAX_BYTES)?;
            let parsed: ItemsResponse = serde_json::from_slice(&body)
                .map_err(|error| HttpError::Parse(bound_error(&error.to_string())))?;
            let mut catalog = Vec::new();
            for item in parsed.data.into_iter().take(CATALOG_ITEM_MAX) {
                let name = item
                    .i18n
                    .and_then(|map| map.get("en").and_then(|entry| entry.name.clone()))
                    .or(item.slug.clone())
                    .unwrap_or_default();
                let slug = item.slug.unwrap_or_default();
                if name.is_empty() || !is_safe_market_slug(&slug) {
                    continue;
                }
                catalog.push(CatalogItem {
                    name: sanitize_item_name(&name),
                    slug,
                });
            }
            self.catalog = Some(catalog);
        }
        Ok(self.catalog.as_deref().unwrap_or_default())
    }
}

fn parse_presence(status: &str) -> TraderPresence {
    match status.to_ascii_lowercase().as_str() {
        "ingame" | "in game" => TraderPresence::Ingame,
        "online" => TraderPresence::Online,
        "offline" => TraderPresence::Offline,
        _ => TraderPresence::Unknown,
    }
}

fn worker_loop<B>(
    publisher: LatestPublisher<MarketSnapshot>,
    commands: Receiver<WorkerCommand>,
    mut backend: B,
    controls: WorkerControls,
    mut diagnostics: ProviderDiagnostics,
    clock: impl Fn() -> u64,
    request_repaint: impl Fn(),
) where
    B: MarketBackend,
{
    let mut error_backoff = ERROR_BACKOFF_INITIAL;
    while let Ok(request) = commands.recv() {
        let is_refresh = request.command == MarketCommand::RefreshSelected;
        if controls.shutdown.load(Ordering::SeqCst) {
            if is_refresh {
                controls.refresh_pending.store(false, Ordering::SeqCst);
            }
            break;
        }
        if !request_is_current(&controls.enabled, &controls.generation, request.generation) {
            if is_refresh {
                controls.refresh_pending.store(false, Ordering::SeqCst);
            }
            continue;
        }
        let mut snapshot = publisher.current().value.as_ref().clone();
        match request.command {
            MarketCommand::Clear => {
                snapshot = MarketSnapshot::default();
                error_backoff = ERROR_BACKOFF_INITIAL;
            }
            MarketCommand::Search(query) => {
                snapshot.query = query.clone();
                snapshot.status = Some("Recherche…".to_owned());
                snapshot.error = None;
                publish_worker_if_current(
                    &publisher,
                    &controls,
                    request.generation,
                    &snapshot,
                    &request_repaint,
                );
                match backend.search(&query) {
                    Ok(results) => {
                        diagnostics.recovered();
                        snapshot.results = results;
                        snapshot.selected = None;
                        snapshot.selected_fetched_at_secs = 0;
                        snapshot.next_refresh_at_secs = 0;
                        snapshot.status = None;
                        error_backoff = ERROR_BACKOFF_INITIAL;
                    }
                    Err(error) => {
                        diagnostics.failed(http_failure_category(&error));
                        snapshot.error = Some(bound_error(&error.to_string()));
                        snapshot.status = None;
                    }
                }
            }
            MarketCommand::Select(slug) => {
                if !is_safe_market_slug(&slug) {
                    diagnostics.failed(FailureCategory::Validation);
                    snapshot.error = Some("invalid item id".to_owned());
                    snapshot.status = None;
                    publish_worker_if_current(
                        &publisher,
                        &controls,
                        request.generation,
                        &snapshot,
                        &request_repaint,
                    );
                    continue;
                }
                snapshot.status = Some("Loading orders…".to_owned());
                snapshot.error = None;
                snapshot.next_refresh_at_secs = u64::MAX;
                publish_worker_if_current(
                    &publisher,
                    &controls,
                    request.generation,
                    &snapshot,
                    &request_repaint,
                );
                match backend.orders(&slug) {
                    Ok(detail) => {
                        diagnostics.recovered();
                        snapshot.selected = Some(detail);
                        let now = clock();
                        snapshot.selected_fetched_at_secs = now;
                        snapshot.next_refresh_at_secs =
                            now.saturating_add(MARKET_ORDERS_REFRESH_SECS);
                        snapshot.error = None;
                        snapshot.status = None;
                        error_backoff = ERROR_BACKOFF_INITIAL;
                    }
                    Err(error) => {
                        diagnostics.failed(http_failure_category(&error));
                        let policy = retry_policy(&error, error_backoff);
                        error_backoff = policy.next_backoff;
                        snapshot.error = Some(bound_error(&error.to_string()));
                        snapshot.status = None;
                        snapshot.next_refresh_at_secs =
                            clock().saturating_add(policy.delay.as_secs());
                    }
                }
            }
            MarketCommand::RefreshSelected => {
                let Some(slug) = snapshot.selected.as_ref().map(|s| s.slug.clone()) else {
                    controls.refresh_pending.store(false, Ordering::SeqCst);
                    continue;
                };
                if !is_safe_market_slug(&slug) {
                    diagnostics.failed(FailureCategory::Validation);
                    controls.refresh_pending.store(false, Ordering::SeqCst);
                    continue;
                }
                // Silent refresh: keep previous data visible; only touch status lightly.
                match backend.orders(&slug) {
                    Ok(detail) => {
                        diagnostics.recovered();
                        snapshot.selected = Some(detail);
                        let now = clock();
                        snapshot.selected_fetched_at_secs = now;
                        snapshot.next_refresh_at_secs =
                            now.saturating_add(MARKET_ORDERS_REFRESH_SECS);
                        snapshot.error = None;
                        snapshot.status = None;
                        error_backoff = ERROR_BACKOFF_INITIAL;
                    }
                    Err(error) => {
                        diagnostics.failed(http_failure_category(&error));
                        let policy = retry_policy(&error, error_backoff);
                        error_backoff = policy.next_backoff;
                        // Keep last good orders; surface a short non-fatal error.
                        snapshot.error = Some(bound_error(&error.to_string()));
                        snapshot.next_refresh_at_secs =
                            clock().saturating_add(policy.delay.as_secs());
                    }
                }
            }
        }
        publish_worker_if_current(
            &publisher,
            &controls,
            request.generation,
            &snapshot,
            &request_repaint,
        );
        if is_refresh {
            controls.refresh_pending.store(false, Ordering::SeqCst);
        }
        if controls.shutdown.load(Ordering::SeqCst) {
            break;
        }
    }
    controls.refresh_pending.store(false, Ordering::SeqCst);
}

fn request_is_current(
    enabled: &AtomicBool,
    generation: &AtomicU64,
    request_generation: u64,
) -> bool {
    enabled.load(Ordering::SeqCst) && generation.load(Ordering::SeqCst) == request_generation
}

fn publish_worker_if_current(
    publisher: &LatestPublisher<MarketSnapshot>,
    controls: &WorkerControls,
    request_generation: u64,
    snapshot: &MarketSnapshot,
    request_repaint: &impl Fn(),
) -> bool {
    let _publication = controls
        .publication_gate
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    publish_if_current(
        publisher,
        &controls.enabled,
        &controls.generation,
        &controls.shutdown,
        request_generation,
        snapshot,
        request_repaint,
    )
}

pub(super) fn publish_if_current(
    publisher: &LatestPublisher<MarketSnapshot>,
    enabled: &AtomicBool,
    generation: &AtomicU64,
    shutdown: &AtomicBool,
    request_generation: u64,
    snapshot: &MarketSnapshot,
    request_repaint: &impl Fn(),
) -> bool {
    if shutdown.load(Ordering::SeqCst)
        || !request_is_current(enabled, generation, request_generation)
        || publisher.current().value.as_ref() == snapshot
    {
        return false;
    }
    if publisher.publish(snapshot.clone()) {
        request_repaint();
    }
    true
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Deserialize)]
struct ItemsResponse {
    #[serde(default)]
    data: Vec<ItemEntry>,
}

#[derive(Deserialize)]
struct ItemEntry {
    slug: Option<String>,
    #[serde(default)]
    i18n: Option<std::collections::HashMap<String, ItemI18n>>,
}

#[derive(Deserialize)]
struct ItemI18n {
    name: Option<String>,
}

#[derive(Deserialize)]
struct OrdersResponse {
    #[serde(default)]
    data: Vec<OrderEntry>,
}

#[derive(Deserialize)]
struct OrderEntry {
    #[serde(rename = "type", default)]
    order_type: String,
    #[serde(default)]
    platinum: u32,
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    user: Option<OrderUser>,
}

#[derive(Deserialize)]
struct OrderUser {
    #[serde(rename = "ingameName", default)]
    ingame_name: Option<String>,
    #[serde(default)]
    status: Option<String>,
}
