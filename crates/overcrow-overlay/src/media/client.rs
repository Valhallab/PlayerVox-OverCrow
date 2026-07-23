use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

use overcrow_logging::EventLogger;
use tokio::sync::mpsc::{Receiver as CommandReceiver, Sender as CommandSender};

use super::{
    model::{MediaAction, MediaCommand, MediaSnapshot},
    mpris::{discover_player, execute_command},
};
use crate::runtime::{
    LatestPublisher, LatestReceiver, VersionedValue, latest_channel,
    widget_diagnostics::{FailureCategory, Provider, ProviderDiagnostics},
};

pub(crate) const WORKER_THREAD_NAME: &str = "overcrow-mpris-provider";
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
pub(crate) const MAXIMUM_BACKOFF: Duration = Duration::from_secs(5);
pub(crate) const COMMAND_CAPACITY: usize = 8;

pub(crate) type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + 'a>>;

pub(crate) trait MediaBackend: Send + 'static {
    fn connect(&mut self) -> BackendFuture<'_, ()>;
    fn discover(&mut self) -> BackendFuture<'_, MediaSnapshot>;
    fn execute<'a>(
        &'a mut self,
        current: &'a MediaSnapshot,
        command: &'a MediaCommand,
    ) -> BackendFuture<'a, ()>;
}

#[derive(Clone, Copy)]
pub(crate) struct WorkerTiming {
    poll_interval: Duration,
    operation_timeout: Duration,
}

impl Default for WorkerTiming {
    fn default() -> Self {
        Self {
            poll_interval: POLL_INTERVAL,
            operation_timeout: OPERATION_TIMEOUT,
        }
    }
}

impl WorkerTiming {
    #[cfg(test)]
    pub(crate) fn for_tests(poll_interval: Duration, operation_timeout: Duration) -> Self {
        Self {
            poll_interval,
            operation_timeout,
        }
    }
}

#[derive(Default)]
struct MprisBackend {
    connection: Option<zbus::Connection>,
}

impl MediaBackend for MprisBackend {
    fn connect(&mut self) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            self.connection = Some(
                zbus::Connection::session()
                    .await
                    .map_err(|error| error.to_string())?,
            );
            Ok(())
        })
    }

    fn discover(&mut self) -> BackendFuture<'_, MediaSnapshot> {
        let Some(connection) = self.connection.as_ref() else {
            return Box::pin(async { Err("MPRIS session bus is not connected".to_owned()) });
        };
        Box::pin(async move {
            discover_player(connection)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn execute<'a>(
        &'a mut self,
        current: &'a MediaSnapshot,
        command: &'a MediaCommand,
    ) -> BackendFuture<'a, ()> {
        let Some(connection) = self.connection.as_ref() else {
            return Box::pin(async { Err("MPRIS session bus is not connected".to_owned()) });
        };
        Box::pin(async move {
            execute_command(connection, current, command)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

pub(crate) type SnapshotPublisher = LatestPublisher<MediaSnapshot>;
pub(crate) type SnapshotReceiver = LatestReceiver<MediaSnapshot>;

pub(crate) fn snapshot_channel() -> (SnapshotPublisher, SnapshotReceiver) {
    latest_channel(MediaSnapshot::default())
}

pub(crate) fn command_channel() -> (CommandSender<MediaCommand>, CommandReceiver<MediaCommand>) {
    tokio::sync::mpsc::channel(COMMAND_CAPACITY)
}

pub struct MediaClient {
    snapshots: SnapshotReceiver,
    commands: CommandSender<MediaCommand>,
    shutdown: tokio::sync::watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}

impl MediaClient {
    pub fn spawn(logger: EventLogger, request_repaint: impl Fn() + Send + Sync + 'static) -> Self {
        Self::spawn_backend(
            MprisBackend::default(),
            logger,
            request_repaint,
            WorkerTiming::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_backend(
        backend: impl MediaBackend,
        request_repaint: impl Fn() + Send + Sync + 'static,
        timing: WorkerTiming,
    ) -> Self {
        Self::spawn_backend(backend, EventLogger::disabled(), request_repaint, timing)
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_backend_and_logger(
        backend: impl MediaBackend,
        logger: EventLogger,
        request_repaint: impl Fn() + Send + Sync + 'static,
        timing: WorkerTiming,
    ) -> Self {
        Self::spawn_backend(backend, logger, request_repaint, timing)
    }

    fn spawn_backend(
        backend: impl MediaBackend,
        logger: EventLogger,
        request_repaint: impl Fn() + Send + Sync + 'static,
        timing: WorkerTiming,
    ) -> Self {
        let (publisher, snapshots) = snapshot_channel();
        let (commands, command_receiver) = command_channel();
        let (shutdown, shutdown_receiver) = tokio::sync::watch::channel(false);
        let fallback_publisher = publisher.clone();
        let request_repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(request_repaint);
        let worker_repaint = Arc::clone(&request_repaint);
        let spawn_logger = logger.clone();
        let worker = thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_owned())
            .spawn(move || {
                run_worker(
                    backend,
                    publisher,
                    command_receiver,
                    shutdown_receiver,
                    move || worker_repaint(),
                    timing,
                    ProviderDiagnostics::new(logger, Provider::Mpris),
                );
            })
            .inspect_err(|error| {
                ProviderDiagnostics::new(spawn_logger, Provider::Mpris)
                    .failed(FailureCategory::Startup);
                publish_spawn_failure(&fallback_publisher, error, request_repaint.as_ref());
            })
            .ok();

        Self {
            snapshots,
            commands,
            shutdown,
            worker,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_parts(
        snapshots: SnapshotReceiver,
        commands: CommandSender<MediaCommand>,
        shutdown: tokio::sync::watch::Sender<bool>,
        worker: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            snapshots,
            commands,
            shutdown,
            worker,
        }
    }

    pub fn take_latest(&self) -> Option<VersionedValue<MediaSnapshot>> {
        self.snapshots.take_latest()
    }

    pub fn send(&self, snapshot: &MediaSnapshot, action: MediaAction) -> bool {
        action
            .command_for(snapshot)
            .is_some_and(|command| self.commands.try_send(command).is_ok())
    }
}

impl Drop for MediaClient {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    backend: impl MediaBackend,
    publisher: SnapshotPublisher,
    commands: CommandReceiver<MediaCommand>,
    shutdown: tokio::sync::watch::Receiver<bool>,
    request_repaint: impl Fn(),
    timing: WorkerTiming,
    mut diagnostics: ProviderDiagnostics,
) {
    let Ok(runtime) = build_runtime() else {
        diagnostics.failed(FailureCategory::Startup);
        publish_if_changed(
            &publisher,
            &MediaSnapshot::provider_error("MPRIS runtime unavailable"),
            &request_repaint,
        );
        return;
    };
    runtime.block_on(run_provider(
        backend,
        publisher,
        commands,
        shutdown,
        request_repaint,
        timing,
        diagnostics,
    ));
}

struct OperationFailure {
    category: FailureCategory,
    message: String,
}

pub(crate) fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

async fn run_provider(
    mut backend: impl MediaBackend,
    publisher: SnapshotPublisher,
    mut commands: CommandReceiver<MediaCommand>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    request_repaint: impl Fn(),
    timing: WorkerTiming,
    mut diagnostics: ProviderDiagnostics,
) {
    let mut backoff = Backoff::new(INITIAL_BACKOFF, MAXIMUM_BACKOFF);
    loop {
        let connection = await_operation(
            "connection",
            FailureCategory::Connection,
            backend.connect(),
            timing.operation_timeout,
            &mut shutdown,
        )
        .await;
        match connection {
            None => return,
            Some(Ok(())) => {}
            Some(Err(error)) => {
                diagnostics.failed(error.category);
                publish_if_changed(
                    &publisher,
                    &MediaSnapshot::provider_error(&error.message),
                    &request_repaint,
                );
                if wait_or_shutdown(backoff.next_delay(), &mut shutdown).await {
                    return;
                }
                continue;
            }
        }

        let result = connection_cycle(
            &mut backend,
            &publisher,
            &mut commands,
            &mut shutdown,
            &request_repaint,
            &mut backoff,
            timing,
            &mut diagnostics,
        )
        .await;
        let Some(error) = result else {
            return;
        };
        diagnostics.failed(error.category);
        publish_if_changed(
            &publisher,
            &MediaSnapshot::provider_error(&error.message),
            &request_repaint,
        );
        if wait_or_shutdown(backoff.next_delay(), &mut shutdown).await {
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn connection_cycle(
    backend: &mut impl MediaBackend,
    publisher: &SnapshotPublisher,
    commands: &mut CommandReceiver<MediaCommand>,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
    request_repaint: &impl Fn(),
    backoff: &mut Backoff,
    timing: WorkerTiming,
    diagnostics: &mut ProviderDiagnostics,
) -> Option<OperationFailure> {
    let mut poll = tokio::time::interval(timing.poll_interval);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    if wait_for_poll(&mut poll, shutdown).await {
        return None;
    }

    loop {
        let current = match await_operation(
            "discovery",
            FailureCategory::Discovery,
            backend.discover(),
            timing.operation_timeout,
            shutdown,
        )
        .await
        {
            None => return None,
            Some(Ok(current)) => current,
            Some(Err(error)) => return Some(error),
        };
        diagnostics.recovered();
        publish_if_changed(publisher, &current, request_repaint);
        backoff.reset();

        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    let _ = changed;
                    return None;
                }
                _ = poll.tick() => break,
                command = commands.recv() => {
                    let command = command?;
                    let result = await_operation(
                        "command",
                        FailureCategory::Command,
                        backend.execute(&current, &command),
                        timing.operation_timeout,
                        shutdown,
                    ).await;
                    match result {
                        None => return None,
                        Some(Ok(())) => {}
                        Some(Err(error)) => diagnostics.failed(error.category),
                    }
                    // Command failures and timeouts stay isolated from healthy polling.
                }
            }
        }
    }
}

async fn await_operation<T>(
    name: &str,
    failure_category: FailureCategory,
    operation: BackendFuture<'_, T>,
    timeout: Duration,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Option<Result<T, OperationFailure>> {
    if shutdown_requested(shutdown) {
        return None;
    }
    tokio::select! {
        biased;
        changed = shutdown.changed() => {
            let _ = changed;
            None
        }
        result = tokio::time::timeout(timeout, operation) => {
            Some(match result {
                Ok(result) => result.map_err(|message| OperationFailure {
                    category: failure_category,
                    message,
                }),
                Err(_) => Err(OperationFailure {
                    category: FailureCategory::Timeout,
                    message: format!("MPRIS {name} timed out"),
                }),
            })
        }
    }
}

async fn wait_for_poll(
    poll: &mut tokio::time::Interval,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    if shutdown_requested(shutdown) {
        return true;
    }
    tokio::select! {
        biased;
        changed = shutdown.changed() => {
            let _ = changed;
            true
        }
        _ = poll.tick() => false,
    }
}

pub(crate) fn publish_if_changed(
    publisher: &SnapshotPublisher,
    snapshot: &MediaSnapshot,
    request_repaint: &(impl Fn() + ?Sized),
) -> bool {
    if publisher.current().value.as_ref() == snapshot {
        return false;
    }
    if publisher.publish(snapshot.clone()) {
        request_repaint();
    }
    true
}

pub(crate) fn publish_spawn_failure(
    publisher: &SnapshotPublisher,
    error: &std::io::Error,
    request_repaint: &(impl Fn() + ?Sized),
) -> bool {
    publish_if_changed(
        publisher,
        &MediaSnapshot::provider_error(&error.to_string()),
        request_repaint,
    )
}

fn shutdown_requested(shutdown: &tokio::sync::watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

async fn wait_or_shutdown(
    delay: Duration,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    if shutdown_requested(shutdown) {
        return true;
    }
    tokio::select! {
        biased;
        changed = shutdown.changed() => {
            let _ = changed;
            true
        }
        () = tokio::time::sleep(delay) => false,
    }
}

pub(crate) struct Backoff {
    initial: Duration,
    maximum: Duration,
    current: Duration,
}

impl Backoff {
    pub(crate) fn new(initial: Duration, maximum: Duration) -> Self {
        let initial = initial.min(maximum);
        Self {
            initial,
            maximum,
            current: initial,
        }
    }

    pub(crate) fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = self
            .current
            .checked_mul(2)
            .unwrap_or(self.maximum)
            .min(self.maximum);
        delay
    }

    fn reset(&mut self) {
        self.current = self.initial;
    }
}
