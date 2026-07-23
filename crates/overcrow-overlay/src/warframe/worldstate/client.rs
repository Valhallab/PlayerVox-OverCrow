use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use overcrow_logging::EventLogger;

use super::parse::parse_worldstate;
use crate::{
    runtime::{
        LatestPublisher, LatestReceiver, VersionedValue, latest_channel,
        widget_diagnostics::{FailureCategory, Provider, ProviderDiagnostics},
    },
    warframe::{
        http::{
            WORLDSTATE_HOST, WORLDSTATE_MAX_BYTES, http_failure_category, https_get_allowlisted,
        },
        model::{ERROR_MAX_CHARS, WorldstateSnapshot, bound_chars},
    },
};

pub const WORLDSTATE_URL: &str = "https://api.warframe.com/cdn/worldState.php";
const POLL_INTERVAL: Duration = Duration::from_secs(60);
const IDLE_INTERVAL: Duration = Duration::from_millis(250);
const ERROR_BACKOFF_INITIAL: Duration = Duration::from_secs(5);
const ERROR_BACKOFF_MAX: Duration = Duration::from_secs(120);
/// Minimum gap between phase-roll forced fetches (UI may request every frame).
const FORCE_REFRESH_MIN_INTERVAL_SECS: u64 = 10;
/// Failed refreshes may retain the last good payload for five minutes.
pub const WORLDSTATE_STALE_TTL_SECS: u64 = 5 * 60;

pub struct WorldstateClient {
    latest: LatestReceiver<WorldstateSnapshot>,
    polling_enabled: Arc<AtomicBool>,
    /// When true, the worker polls on the next idle tick (phase roll).
    refresh_requested: Arc<AtomicBool>,
    last_force_refresh_secs: Arc<AtomicU64>,
    shutdown: Option<mpsc::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl Default for WorldstateClient {
    fn default() -> Self {
        Self::new(EventLogger::disabled(), || {})
    }
}

impl WorldstateClient {
    pub fn new(logger: EventLogger, request_repaint: impl Fn() + Send + 'static) -> Self {
        Self::spawn(default_fetch, logger, request_repaint)
    }

    #[cfg(test)]
    pub fn with_fetcher<F, R>(fetcher: F, request_repaint: R) -> Self
    where
        F: Fn() -> Result<Vec<u8>, String> + Send + 'static,
        R: Fn() + Send + 'static,
    {
        Self::spawn(
            move || {
                fetcher().map_err(|message| FetchFailure {
                    category: FailureCategory::Transport,
                    message,
                })
            },
            EventLogger::disabled(),
            request_repaint,
        )
    }

    #[cfg(test)]
    pub fn with_fetcher_and_logger<F, R>(
        fetcher: F,
        logger: EventLogger,
        request_repaint: R,
    ) -> Self
    where
        F: Fn() -> Result<Vec<u8>, String> + Send + 'static,
        R: Fn() + Send + 'static,
    {
        Self::spawn(
            move || {
                fetcher().map_err(|message| FetchFailure {
                    category: FailureCategory::Transport,
                    message,
                })
            },
            logger,
            request_repaint,
        )
    }

    fn spawn<F, R>(fetcher: F, logger: EventLogger, request_repaint: R) -> Self
    where
        F: Fn() -> Result<Vec<u8>, FetchFailure> + Send + 'static,
        R: Fn() + Send + 'static,
    {
        let (publisher, latest) = latest_channel(WorldstateSnapshot::default());
        let polling_enabled = Arc::new(AtomicBool::new(false));
        let refresh_requested = Arc::new(AtomicBool::new(false));
        let last_force_refresh_secs = Arc::new(AtomicU64::new(0));
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let worker_enabled = Arc::clone(&polling_enabled);
        let worker_refresh = Arc::clone(&refresh_requested);
        let join = thread::Builder::new()
            .name("overcrow-warframe-worldstate".to_owned())
            .spawn(move || {
                worker_loop(
                    publisher,
                    worker_enabled,
                    worker_refresh,
                    shutdown_rx,
                    fetcher,
                    request_repaint,
                    ProviderDiagnostics::new(logger, Provider::WarframeWorldstate),
                );
            })
            .expect("spawn worldstate worker");
        Self {
            latest,
            polling_enabled,
            refresh_requested,
            last_force_refresh_secs,
            shutdown: Some(shutdown_tx),
            join: Some(join),
        }
    }

    pub fn set_polling_enabled(&self, enabled: bool) {
        self.polling_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Re-fetch soon (coalesced; at most once per [`FORCE_REFRESH_MIN_INTERVAL_SECS`]).
    pub fn request_refresh(&self) {
        let now = now_secs();
        let last = self.last_force_refresh_secs.load(Ordering::Relaxed);
        if now.saturating_sub(last) < FORCE_REFRESH_MIN_INTERVAL_SECS
            || self.refresh_requested.load(Ordering::Relaxed)
        {
            return;
        }
        self.last_force_refresh_secs.store(now, Ordering::Relaxed);
        self.refresh_requested.store(true, Ordering::Relaxed);
    }

    pub fn take_latest(&self) -> Option<VersionedValue<WorldstateSnapshot>> {
        self.latest.take_latest()
    }
}

struct FetchFailure {
    category: FailureCategory,
    message: String,
}

impl Drop for WorldstateClient {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(join) = self.join.take() {
            // Bound shutdown: worker times out HTTP itself.
            let _ = join.join();
        }
    }
}

fn worker_loop<F, R>(
    publisher: LatestPublisher<WorldstateSnapshot>,
    polling_enabled: Arc<AtomicBool>,
    refresh_requested: Arc<AtomicBool>,
    shutdown: mpsc::Receiver<()>,
    fetcher: F,
    request_repaint: R,
    mut diagnostics: ProviderDiagnostics,
) where
    F: Fn() -> Result<Vec<u8>, FetchFailure>,
    R: Fn(),
{
    let mut published = publisher.current().value;
    let mut last_good = Arc::clone(&published);
    let mut next_poll = SystemTime::now();
    let mut error_backoff = ERROR_BACKOFF_INITIAL;
    loop {
        if shutdown.try_recv().is_ok() {
            break;
        }
        let enabled = polling_enabled.load(Ordering::Relaxed);
        let forced = refresh_requested.swap(false, Ordering::Relaxed);
        if enabled && (forced || SystemTime::now() >= next_poll) {
            let fetched_at_secs = now_secs();
            match fetcher() {
                Ok(bytes) => match parse_worldstate(&bytes, fetched_at_secs) {
                    Ok(snapshot) => {
                        diagnostics.recovered();
                        if publish_if_changed(
                            &publisher,
                            &mut published,
                            snapshot,
                            &request_repaint,
                        ) {
                            last_good = Arc::clone(&published);
                        }
                        error_backoff = ERROR_BACKOFF_INITIAL;
                        next_poll = SystemTime::now() + POLL_INTERVAL;
                    }
                    Err(error) => {
                        diagnostics.failed(FailureCategory::Parse);
                        let snapshot = failure_snapshot(
                            &last_good,
                            fetched_at_secs,
                            bound_chars(&error.to_string(), ERROR_MAX_CHARS),
                        );
                        publish_if_changed(&publisher, &mut published, snapshot, &request_repaint);
                        next_poll = SystemTime::now() + error_backoff;
                        error_backoff = (error_backoff * 2).min(ERROR_BACKOFF_MAX);
                    }
                },
                Err(error) => {
                    diagnostics.failed(error.category);
                    let snapshot = failure_snapshot(
                        &last_good,
                        fetched_at_secs,
                        bound_chars(&error.message, ERROR_MAX_CHARS),
                    );
                    publish_if_changed(&publisher, &mut published, snapshot, &request_repaint);
                    next_poll = SystemTime::now() + error_backoff;
                    error_backoff = (error_backoff * 2).min(ERROR_BACKOFF_MAX);
                }
            }
        }
        if shutdown.recv_timeout(IDLE_INTERVAL).is_ok() {
            break;
        }
    }
}

pub(super) fn publish_if_changed(
    publisher: &LatestPublisher<WorldstateSnapshot>,
    published: &mut Arc<WorldstateSnapshot>,
    snapshot: WorldstateSnapshot,
    request_repaint: &impl Fn(),
) -> bool {
    if published.as_ref() == &snapshot {
        return false;
    }
    let receiver_connected = publisher.publish(snapshot);
    *published = publisher.current().value;
    if receiver_connected {
        request_repaint();
    }
    true
}

/// Retain the last good payload for a fixed grace period, without misdating it as fresh.
fn failure_snapshot(
    last_good: &WorldstateSnapshot,
    attempted_at_secs: u64,
    error: String,
) -> WorldstateSnapshot {
    let fetched_at_secs = last_good.fetched_at_secs;
    if last_good.has_payload()
        && attempted_at_secs.saturating_sub(fetched_at_secs) > WORLDSTATE_STALE_TTL_SECS
    {
        WorldstateSnapshot {
            fetched_at_secs,
            last_attempt_at_secs: attempted_at_secs,
            error: Some(error),
            ..WorldstateSnapshot::default()
        }
    } else {
        let mut snapshot = last_good.clone();
        snapshot.last_attempt_at_secs = attempted_at_secs;
        snapshot.error = Some(error);
        snapshot
    }
}

fn default_fetch() -> Result<Vec<u8>, FetchFailure> {
    https_get_allowlisted(WORLDSTATE_URL, WORLDSTATE_HOST, WORLDSTATE_MAX_BYTES).map_err(|error| {
        FetchFailure {
            category: http_failure_category(&error),
            message: error.to_string(),
        }
    })
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use overcrow_logging::{Component, LoggerRuntime};

    use super::{WORLDSTATE_STALE_TTL_SECS, WorldstateClient};
    use crate::warframe::model::{CycleStatus, WorldstateSnapshot};

    #[test]
    fn disabled_worker_does_not_fetch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_worker = Arc::clone(&calls);
        let client = WorldstateClient::with_fetcher(
            move || {
                calls_worker.fetch_add(1, Ordering::Relaxed);
                Err("no".to_owned())
            },
            || {},
        );
        thread::sleep(Duration::from_millis(400));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        drop(client);
    }

    #[test]
    fn enabled_worker_records_fetch_errors() {
        let client = WorldstateClient::with_fetcher(|| Err("network down".to_owned()), || {});
        client.set_polling_enabled(true);
        thread::sleep(Duration::from_millis(500));
        let snapshot = client
            .take_latest()
            .expect("failed fetch publishes a snapshot")
            .value;
        assert!(
            snapshot
                .error
                .as_deref()
                .is_some_and(|message| message.contains("network"))
        );
    }

    #[test]
    fn warframe_diagnostic_worldstate_classifies_fetch_and_parse_without_private_text() {
        for (fetcher, category) in [
            (
                (|| Err("private fetch detail".to_owned())) as fn() -> Result<Vec<u8>, String>,
                "transport",
            ),
            (|| Ok(b"{".to_vec()), "parse"),
        ] {
            let temp = tempfile::tempdir().expect("create log directory");
            let log_runtime = LoggerRuntime::start_in(Component::Overlay, temp.path())
                .expect("start test logger");
            let client =
                WorldstateClient::with_fetcher_and_logger(fetcher, log_runtime.logger(), || {});
            client.set_polling_enabled(true);

            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while client.take_latest().is_none() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "worldstate failure was not published"
                );
                thread::sleep(Duration::from_millis(10));
            }
            drop(client);
            drop(log_runtime);

            let contents = std::fs::read_to_string(temp.path().join("overlay.log"))
                .expect("read diagnostic log");
            assert!(contents.contains(&format!(
                "provider=warframe_worldstate affected_widgets=warframe_status,warframe_fissures,warframe_sortie,warframe_invasions category={category}"
            )));
            assert!(!contents.contains("private fetch detail"));
        }
    }

    #[test]
    fn failed_refresh_keeps_previous_payload() {
        use super::failure_snapshot;

        let last_good = WorldstateSnapshot {
            server_time_secs: 1000,
            fetched_at_secs: 1000,
            cycles: vec![CycleStatus {
                id: "cetus".to_owned(),
                label: "Cetus".to_owned(),
                state: Some("day".to_owned()),
                expires_at_secs: 2000,
            }],
            daily_reset_at_secs: Some(2000),
            ..WorldstateSnapshot::default()
        };
        let after = failure_snapshot(&last_good, 1100, "boom".to_owned());
        assert_eq!(after.server_time_secs, 1000);
        assert_eq!(after.cycles.len(), 1);
        assert_eq!(after.error.as_deref(), Some("boom"));
        assert_eq!(after.fetched_at_secs, 1000);
        assert_eq!(after.last_attempt_at_secs, 1100);
        assert!(after.has_payload());
    }

    #[test]
    fn failed_refresh_at_ttl_boundary_keeps_previous_payload() {
        use super::failure_snapshot;

        let last_good = WorldstateSnapshot {
            server_time_secs: 1000,
            fetched_at_secs: 1000,
            cycles: vec![CycleStatus {
                id: "cetus".to_owned(),
                label: "Cetus".to_owned(),
                state: Some("day".to_owned()),
                expires_at_secs: 2000,
            }],
            ..WorldstateSnapshot::default()
        };
        let attempted_at_secs = 1000 + WORLDSTATE_STALE_TTL_SECS;

        let after = failure_snapshot(&last_good, attempted_at_secs, "still down".to_owned());
        assert_eq!(after.fetched_at_secs, 1000);
        assert_eq!(after.last_attempt_at_secs, attempted_at_secs);
        assert_eq!(after.error.as_deref(), Some("still down"));
        assert!(after.has_payload());
    }

    #[test]
    fn failed_refresh_after_ttl_clears_payload_but_preserves_freshness_metadata() {
        use super::failure_snapshot;

        let last_good = WorldstateSnapshot {
            server_time_secs: 1000,
            fetched_at_secs: 1000,
            cycles: vec![CycleStatus {
                id: "cetus".to_owned(),
                label: "Cetus".to_owned(),
                state: Some("day".to_owned()),
                expires_at_secs: 2000,
            }],
            ..WorldstateSnapshot::default()
        };
        let attempted_at_secs = 1000 + WORLDSTATE_STALE_TTL_SECS + 1;

        let after = failure_snapshot(&last_good, attempted_at_secs, "still down".to_owned());
        assert_eq!(after.fetched_at_secs, 1000);
        assert_eq!(after.last_attempt_at_secs, attempted_at_secs);
        assert_eq!(after.error.as_deref(), Some("still down"));
        assert!(!after.has_payload());
    }
}
