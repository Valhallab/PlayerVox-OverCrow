use futures_util::StreamExt;
use overcrow_logging::EventLogger;
use overcrow_protocol::{Core1Proxy, CoreSnapshot, VersionedCoreSnapshot};
use std::{
    fmt::{self, Write as _},
    future::Future,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};
use zbus::proxy::ProxyImpl;

const LEGACY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const RECONCILIATION_INTERVAL: Duration = Duration::from_secs(30);
#[cfg(test)]
const COMMAND_CHANNEL_CAPACITY: usize = 1;
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAXIMUM_BACKOFF: Duration = Duration::from_secs(5);
const MAX_CONNECTION_ERROR_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq)]
// The required decision API returns the accepted envelope directly so callers
// cannot publish a different value from the one admitted by the revision gate.
#[allow(clippy::large_enum_variant)]
enum RevisionDecision {
    Apply(VersionedCoreSnapshot),
    Ignore,
    Reconcile,
}

#[derive(Debug, Default)]
struct RevisionGate {
    applied: Option<VersionedCoreSnapshot>,
}

impl RevisionGate {
    fn apply(&mut self, event: VersionedCoreSnapshot) -> RevisionDecision {
        let Some(applied) = &self.applied else {
            self.applied = Some(event.clone());
            return RevisionDecision::Apply(event);
        };

        if event.revision < applied.revision {
            RevisionDecision::Ignore
        } else if event.revision == applied.revision {
            if event.snapshot == applied.snapshot {
                RevisionDecision::Ignore
            } else {
                RevisionDecision::Reconcile
            }
        } else {
            self.applied = Some(event.clone());
            RevisionDecision::Apply(event)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VersionedHandling {
    Applied,
    Ignored,
    Reconcile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnerStreamEvent {
    Changed,
    Ended,
}

impl OwnerStreamEvent {
    fn from_next<T>(next: Option<T>) -> Self {
        if next.is_some() {
            Self::Changed
        } else {
            Self::Ended
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionGenerationDecision {
    Reconnect,
}

fn owner_stream_decision(_event: OwnerStreamEvent) -> ConnectionGenerationDecision {
    ConnectionGenerationDecision::Reconnect
}

fn owner_stream_error<T>(next: Option<T>) -> zbus::Error {
    let event = OwnerStreamEvent::from_next(next);
    match owner_stream_decision(event) {
        ConnectionGenerationDecision::Reconnect => {
            let detail = match event {
                OwnerStreamEvent::Changed => "Core D-Bus owner changed",
                OwnerStreamEvent::Ended => "Core D-Bus owner-change stream ended",
            };
            zbus::Error::Failure(detail.to_owned())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ConnectionEvent {
    Connected,
    ConnectionFailed(String),
    Disconnected(String),
}

impl ConnectionEvent {
    fn emit(&self, logger: &EventLogger) {
        match self {
            Self::Connected => logger.info("core_connected", format_args!("")),
            Self::ConnectionFailed(error) => {
                logger.warn("core_connection_failed", format_args!("error={error:?}"))
            }
            Self::Disconnected(error) => {
                logger.warn("core_disconnected", format_args!("error={error:?}"));
            }
        }
    }
}

#[derive(Debug, Default)]
struct ConnectionEventTracker {
    connected: bool,
    last_failure: Option<String>,
}

impl ConnectionEventTracker {
    fn connected(&mut self) -> Option<ConnectionEvent> {
        if self.connected {
            return None;
        }
        self.connected = true;
        self.last_failure = None;
        Some(ConnectionEvent::Connected)
    }

    fn failed(&mut self, error: impl fmt::Display) -> Option<ConnectionEvent> {
        let error = bounded_display(error, MAX_CONNECTION_ERROR_BYTES);
        if !self.connected && self.last_failure.as_ref() == Some(&error) {
            return None;
        }
        let event = if self.connected {
            ConnectionEvent::Disconnected(error.clone())
        } else {
            ConnectionEvent::ConnectionFailed(error.clone())
        };
        self.connected = false;
        self.last_failure = Some(error);
        Some(event)
    }
}

struct BoundedDisplay {
    value: String,
    maximum: usize,
}

impl fmt::Write for BoundedDisplay {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.value.len() >= self.maximum {
            return Ok(());
        }
        let mut end = value.len().min(self.maximum - self.value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.value.push_str(&value[..end]);
        Ok(())
    }
}

fn bounded_display(value: impl fmt::Display, maximum: usize) -> String {
    let mut output = BoundedDisplay {
        value: String::with_capacity(maximum),
        maximum,
    };
    let _ = write!(output, "{value}");
    output.value
}

async fn until_owner_change<T>(
    owner_changes: &mut zbus::proxy::OwnerChangedStream<'_>,
    operation: impl Future<Output = zbus::Result<T>>,
) -> zbus::Result<T> {
    tokio::select! {
        biased;
        owner_change = owner_changes.next() => Err(owner_stream_error(owner_change)),
        result = operation => result,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BaselineFailure {
    Legacy,
    Reconnect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandResponseMode {
    Versioned,
    Legacy,
}

fn baseline_failure_for_error_name(name: Option<&str>) -> BaselineFailure {
    if name == Some("org.freedesktop.DBus.Error.UnknownMethod") {
        BaselineFailure::Legacy
    } else {
        BaselineFailure::Reconnect
    }
}

fn baseline_failure(error: &zbus::Error) -> BaselineFailure {
    let name = match error {
        zbus::Error::MethodError(name, _, _) => Some(name.as_str()),
        _ => None,
    };
    baseline_failure_for_error_name(name)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClientCommand {
    SetPassive,
    ReloadWidgetSettings,
    ToggleManualStopwatch,
    ResetManualStopwatch,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PendingWidgetActions {
    reload: bool,
    reset: bool,
    toggle: bool,
}

impl PendingWidgetActions {
    fn push(&mut self, command: ClientCommand) {
        match command {
            ClientCommand::SetPassive => {}
            ClientCommand::ReloadWidgetSettings => self.reload = true,
            ClientCommand::ToggleManualStopwatch => self.toggle = !self.toggle,
            ClientCommand::ResetManualStopwatch => {
                self.reset = true;
                self.toggle = false;
            }
        }
    }

    fn merge_newer(&mut self, newer: Self) {
        self.reload |= newer.reload;
        if newer.reset {
            self.reset = true;
            self.toggle = newer.toggle;
        } else {
            self.toggle ^= newer.toggle;
        }
    }

    fn take_next(&mut self) -> Option<ClientCommand> {
        if std::mem::take(&mut self.reload) {
            Some(ClientCommand::ReloadWidgetSettings)
        } else if std::mem::take(&mut self.reset) {
            Some(ClientCommand::ResetManualStopwatch)
        } else if std::mem::take(&mut self.toggle) {
            Some(ClientCommand::ToggleManualStopwatch)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PendingCommands {
    set_passive: bool,
    widget_actions: PendingWidgetActions,
}

impl PendingCommands {
    fn push(&mut self, command: ClientCommand) {
        if command == ClientCommand::SetPassive {
            self.set_passive = true;
        } else {
            self.widget_actions.push(command);
        }
    }
}

struct CommandSender {
    slot: Arc<Mutex<PendingCommands>>,
    ready: Arc<tokio::sync::Notify>,
    open: Arc<AtomicBool>,
}

struct CommandReceiver {
    slot: Arc<Mutex<PendingCommands>>,
    ready: Arc<tokio::sync::Notify>,
    open: Arc<AtomicBool>,
}

fn command_channel() -> (CommandSender, CommandReceiver) {
    let slot = Arc::new(Mutex::new(PendingCommands::default()));
    let ready = Arc::new(tokio::sync::Notify::new());
    let open = Arc::new(AtomicBool::new(true));
    (
        CommandSender {
            slot: Arc::clone(&slot),
            ready: Arc::clone(&ready),
            open: Arc::clone(&open),
        },
        CommandReceiver { slot, ready, open },
    )
}

impl CommandSender {
    fn send(&self, command: ClientCommand) -> bool {
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(command);
        self.ready.notify_one();
        true
    }
}

impl Drop for CommandSender {
    fn drop(&mut self) {
        self.open.store(false, Ordering::Release);
        self.ready.notify_one();
    }
}

impl CommandReceiver {
    async fn notified(&self) {
        self.ready.notified().await;
    }

    fn take(&self) -> PendingCommands {
        std::mem::take(&mut *self.slot.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotUpdate {
    pub snapshot: CoreSnapshot,
    pub passive_confirmed: bool,
    confirmed: bool,
}

impl SnapshotUpdate {
    pub fn confirmed(snapshot: CoreSnapshot, passive_confirmed: bool) -> Self {
        Self {
            snapshot,
            passive_confirmed,
            confirmed: true,
        }
    }

    pub fn unconfirmed(snapshot: CoreSnapshot) -> Self {
        Self {
            snapshot,
            passive_confirmed: false,
            confirmed: false,
        }
    }

    pub fn is_confirmed(&self) -> bool {
        self.confirmed
    }
}

#[derive(Clone)]
struct SnapshotSender {
    slot: Arc<Mutex<Option<SnapshotUpdate>>>,
    ready: SyncSender<()>,
}

struct SnapshotReceiver {
    slot: Arc<Mutex<Option<SnapshotUpdate>>>,
    ready: Receiver<()>,
}

fn snapshot_channel() -> (SnapshotSender, SnapshotReceiver) {
    let slot = Arc::new(Mutex::new(None));
    let (ready, receiver) = mpsc::sync_channel(1);
    (
        SnapshotSender {
            slot: Arc::clone(&slot),
            ready,
        },
        SnapshotReceiver {
            slot,
            ready: receiver,
        },
    )
}

impl SnapshotSender {
    fn publish(&self, mut update: SnapshotUpdate) -> bool {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(previous) = slot.take() {
            update.passive_confirmed |= previous.passive_confirmed;
        }
        *slot = Some(update);
        drop(slot);

        match self.ready.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => true,
            Err(TrySendError::Disconnected(())) => false,
        }
    }
}

impl SnapshotReceiver {
    fn take_latest(&self) -> Option<SnapshotUpdate> {
        self.ready.try_recv().ok()?;
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

#[derive(Debug, Default)]
struct PassiveIntent {
    pending: bool,
    widget_actions: PendingWidgetActions,
}

impl PassiveIntent {
    #[cfg(test)]
    fn pending() -> Self {
        Self {
            pending: true,
            widget_actions: PendingWidgetActions::default(),
        }
    }

    fn absorb_commands(&mut self, commands: &CommandReceiver) -> bool {
        let incoming = commands.take();
        self.pending |= incoming.set_passive;
        self.widget_actions.merge_newer(incoming.widget_actions);
        commands.is_open()
    }

    fn should_request(&self) -> bool {
        self.pending
    }

    fn next_widget_action(&mut self) -> Option<ClientCommand> {
        // Dispatch actions at most once: retrying an ambiguously completed toggle
        // after reconnect could apply the opposite transition.
        self.widget_actions.take_next()
    }

    fn record_response(&mut self, response: Result<&CoreSnapshot, ()>) {
        if response
            .is_ok_and(|snapshot| snapshot.overlay_mode == overcrow_protocol::OverlayMode::Passive)
        {
            self.pending = false;
        }
    }

    fn record_snapshot(&mut self, snapshot: &CoreSnapshot) {
        if snapshot.overlay_mode == overcrow_protocol::OverlayMode::Passive {
            self.pending = false;
        }
    }
}

pub struct SnapshotClient {
    snapshots: SnapshotReceiver,
    commands: CommandSender,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl SnapshotClient {
    pub fn spawn(logger: EventLogger, request_repaint: impl Fn() + Send + 'static) -> Self {
        let (snapshot_sender, snapshots) = snapshot_channel();
        let (commands, command_receiver) = command_channel();
        let (shutdown, shutdown_receiver) = tokio::sync::watch::channel(false);

        let _ = thread::Builder::new()
            .name("overcrow-dbus-client".to_owned())
            .spawn(move || {
                run_worker(
                    snapshot_sender,
                    command_receiver,
                    shutdown_receiver,
                    logger,
                    request_repaint,
                );
            });

        Self {
            snapshots,
            commands,
            shutdown,
        }
    }

    #[cfg(test)]
    fn from_channels(snapshots: SnapshotReceiver, commands: CommandSender) -> Self {
        let (shutdown, _shutdown_receiver) = tokio::sync::watch::channel(false);
        Self::from_channels_with_shutdown(snapshots, commands, shutdown)
    }

    #[cfg(test)]
    fn from_channels_with_shutdown(
        snapshots: SnapshotReceiver,
        commands: CommandSender,
        shutdown: tokio::sync::watch::Sender<bool>,
    ) -> Self {
        Self {
            snapshots,
            commands,
            shutdown,
        }
    }

    pub fn take_latest(&self) -> Option<SnapshotUpdate> {
        self.snapshots.take_latest()
    }

    /// Retained for the UI API; Core state changes now arrive through signals.
    pub fn set_manual_stopwatch_running(&self, _running: bool) {}

    pub fn request_passive(&self) {
        let _ = self.commands.send(ClientCommand::SetPassive);
    }

    pub fn reload_widget_settings(&self) {
        let _ = self.commands.send(ClientCommand::ReloadWidgetSettings);
    }

    pub fn toggle_manual_stopwatch(&self) {
        let _ = self.commands.send(ClientCommand::ToggleManualStopwatch);
    }

    pub fn reset_manual_stopwatch(&self) {
        let _ = self.commands.send(ClientCommand::ResetManualStopwatch);
    }
}

impl Drop for SnapshotClient {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

fn run_worker(
    snapshots: SnapshotSender,
    commands: CommandReceiver,
    shutdown: tokio::sync::watch::Receiver<bool>,
    logger: EventLogger,
    request_repaint: impl Fn(),
) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        publish_update(
            SnapshotUpdate::unconfirmed(CoreSnapshot::default()),
            &snapshots,
            &request_repaint,
        );
        return;
    };
    runtime.block_on(run_client(
        snapshots,
        commands,
        shutdown,
        logger,
        request_repaint,
    ));
}

async fn run_client(
    snapshots: SnapshotSender,
    commands: CommandReceiver,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    logger: EventLogger,
    request_repaint: impl Fn(),
) {
    let mut backoff = Backoff::new(INITIAL_BACKOFF, MAXIMUM_BACKOFF);
    let mut passive_intent = PassiveIntent::default();
    let mut connection_events = ConnectionEventTracker::default();
    loop {
        let result = tokio::select! {
            () = wait_for_shutdown(&mut shutdown) => return,
            result = connection_cycle(
                &snapshots,
                &commands,
                &request_repaint,
                &mut backoff,
                &mut passive_intent,
                &mut connection_events,
                &logger,
            ) => result,
        };
        if let Err(error) = result
            && let Some(event) = connection_events.failed(error)
        {
            event.emit(&logger);
        }
        // Keep the last good snapshot while reconnecting. Publishing an empty
        // default would blank the UI and leave Hyprland game-input-blocked tags
        // in place if Core is still Interactive.
        tokio::select! {
            () = wait_for_shutdown(&mut shutdown) => return,
            () = tokio::time::sleep(backoff.next_delay()) => {}
        }
    }
}

async fn wait_for_shutdown(shutdown: &mut tokio::sync::watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow() {
            return;
        }
    }
}

async fn connection_cycle(
    snapshots: &SnapshotSender,
    commands: &CommandReceiver,
    request_repaint: &impl Fn(),
    backoff: &mut Backoff,
    passive_intent: &mut PassiveIntent,
    connection_events: &mut ConnectionEventTracker,
    logger: &EventLogger,
) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let proxy = Core1Proxy::new(&connection).await?;
    let mut owner_changes = ProxyImpl::inner(&proxy).receive_owner_changed().await?;
    let mut signals =
        until_owner_change(&mut owner_changes, proxy.receive_snapshot_changed()).await?;

    let baseline_json =
        match until_owner_change(&mut owner_changes, proxy.snapshot_versioned()).await {
            Ok(json) => json,
            Err(error) if baseline_failure(&error) == BaselineFailure::Legacy => {
                drop(owner_changes);
                drop(signals);
                if let Some(event) = connection_events.connected() {
                    event.emit(logger);
                }
                return legacy_connection_cycle(
                    &proxy,
                    snapshots,
                    commands,
                    request_repaint,
                    backoff,
                    passive_intent,
                )
                .await;
            }
            Err(error) => return Err(error),
        };
    let baseline = decode_versioned(&baseline_json).map_err(malformed_versioned_error)?;
    if let Some(event) = connection_events.connected() {
        event.emit(logger);
    }
    let mut gate = RevisionGate::default();
    let _ = apply_versioned(
        baseline,
        &mut gate,
        snapshots,
        passive_intent,
        request_repaint,
    );
    backoff.reset();

    if !passive_intent.absorb_commands(commands) {
        return Ok(());
    }
    until_owner_change(
        &mut owner_changes,
        dispatch_versioned_commands(&proxy, passive_intent, snapshots, request_repaint),
    )
    .await?;

    let reconciliation = tokio::time::sleep(RECONCILIATION_INTERVAL);
    tokio::pin!(reconciliation);

    loop {
        tokio::select! {
            biased;
            owner_change = owner_changes.next() => {
                return Err(owner_stream_error(owner_change));
            }
            signal = signals.next() => {
                let Some(signal) = signal else {
                    return Err(zbus::Error::Failure(
                        "Core SnapshotChanged signal stream ended".to_owned(),
                    ));
                };
                let handling = match signal.args() {
                    Ok(args) => handle_signal_json(
                        args.snapshot_json(),
                        &mut gate,
                        snapshots,
                        passive_intent,
                        request_repaint,
                    ),
                    Err(error) => {
                        eprintln!("OverCrow: malformed Core snapshot signal arguments: {error}");
                        VersionedHandling::Reconcile
                    }
                };
                if handling == VersionedHandling::Reconcile {
                    reconciliation.as_mut().reset(tokio::time::Instant::now());
                }
            }
            () = commands.notified() => {
                if !passive_intent.absorb_commands(commands) {
                    return Ok(());
                }
                until_owner_change(
                    &mut owner_changes,
                    dispatch_versioned_commands(
                        &proxy,
                        passive_intent,
                        snapshots,
                        request_repaint,
                    ),
                )
                .await?;
            }
            () = &mut reconciliation => {
                let json = until_owner_change(
                    &mut owner_changes,
                    proxy.snapshot_versioned(),
                )
                .await?;
                let event = decode_versioned(&json).map_err(malformed_versioned_error)?;
                if apply_versioned(
                    event,
                    &mut gate,
                    snapshots,
                    passive_intent,
                    request_repaint,
                ) == VersionedHandling::Reconcile
                {
                    return Err(zbus::Error::Failure(
                        "Core returned conflicting content for one snapshot revision".to_owned(),
                    ));
                }
                backoff.reset();
                reconciliation
                    .as_mut()
                    .reset(tokio::time::Instant::now() + RECONCILIATION_INTERVAL);
            }
        }
    }
}

async fn dispatch_versioned_commands(
    proxy: &Core1Proxy<'_>,
    passive_intent: &mut PassiveIntent,
    snapshots: &SnapshotSender,
    request_repaint: &impl Fn(),
) -> zbus::Result<()> {
    if passive_intent.should_request() {
        let json = proxy.set_overlay_interactive(false).await?;
        let _ = handle_command_response(
            CommandResponseMode::Versioned,
            &json,
            snapshots,
            request_repaint,
        );
    }

    while let Some(action) = passive_intent.next_widget_action() {
        let json = match action {
            ClientCommand::ReloadWidgetSettings => proxy.reload_widget_settings().await?,
            ClientCommand::ToggleManualStopwatch => proxy.toggle_manual_stopwatch().await?,
            ClientCommand::ResetManualStopwatch => proxy.reset_manual_stopwatch().await?,
            ClientCommand::SetPassive => unreachable!("passive actions are handled separately"),
        };
        let _ = handle_command_response(
            CommandResponseMode::Versioned,
            &json,
            snapshots,
            request_repaint,
        );
    }
    Ok(())
}

async fn legacy_connection_cycle(
    proxy: &Core1Proxy<'_>,
    snapshots: &SnapshotSender,
    commands: &CommandReceiver,
    request_repaint: &impl Fn(),
    backoff: &mut Backoff,
    passive_intent: &mut PassiveIntent,
) -> zbus::Result<()> {
    loop {
        if !passive_intent.absorb_commands(commands) {
            return Ok(());
        }
        if passive_intent.should_request() {
            let json = proxy.set_overlay_interactive(false).await?;
            let response = handle_command_response(
                CommandResponseMode::Legacy,
                &json,
                snapshots,
                request_repaint,
            );
            passive_intent.record_response(response.as_ref().ok_or(()));
            if response.is_some() {
                backoff.reset();
            }
        }

        while let Some(action) = passive_intent.next_widget_action() {
            let json = match action {
                ClientCommand::ReloadWidgetSettings => proxy.reload_widget_settings().await?,
                ClientCommand::ToggleManualStopwatch => proxy.toggle_manual_stopwatch().await?,
                ClientCommand::ResetManualStopwatch => proxy.reset_manual_stopwatch().await?,
                ClientCommand::SetPassive => unreachable!("passive actions are handled separately"),
            };
            if handle_command_response(
                CommandResponseMode::Legacy,
                &json,
                snapshots,
                request_repaint,
            )
            .is_some()
            {
                backoff.reset();
            }
        }

        let json = proxy.snapshot().await?;
        let response = publish_json(&json, snapshots, request_repaint);
        passive_intent.record_response(response.as_ref().ok_or(()));
        if response.is_some() {
            backoff.reset();
        }

        tokio::select! {
            () = commands.notified() => {}
            () = tokio::time::sleep(LEGACY_POLL_INTERVAL) => {}
        }
    }
}

fn malformed_versioned_error(error: serde_json::Error) -> zbus::Error {
    let detail: String = error.to_string().chars().take(256).collect();
    zbus::Error::Failure(format!("malformed versioned Core snapshot: {detail}"))
}

struct Backoff {
    initial: Duration,
    maximum: Duration,
    current: Duration,
}

impl Backoff {
    fn new(initial: Duration, maximum: Duration) -> Self {
        let initial = initial.min(maximum);
        Self {
            initial,
            maximum,
            current: initial,
        }
    }

    fn next_delay(&mut self) -> Duration {
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

fn publish_json(
    json: &str,
    sender: &SnapshotSender,
    request_repaint: impl FnOnce(),
) -> Option<CoreSnapshot> {
    let Ok(snapshot) = serde_json::from_str::<CoreSnapshot>(json) else {
        // Malformed payloads must not wipe the last good interactive session.
        eprintln!("OverCrow: ignoring malformed Core snapshot JSON");
        request_repaint();
        return None;
    };
    let passive_confirmed = snapshot.overlay_mode == overcrow_protocol::OverlayMode::Passive;
    publish_update(
        SnapshotUpdate::confirmed(snapshot.clone(), passive_confirmed),
        sender,
        request_repaint,
    );
    Some(snapshot)
}

fn handle_command_response(
    mode: CommandResponseMode,
    json: &str,
    sender: &SnapshotSender,
    request_repaint: impl FnOnce(),
) -> Option<CoreSnapshot> {
    match mode {
        CommandResponseMode::Versioned => None,
        CommandResponseMode::Legacy => publish_json(json, sender, request_repaint),
    }
}

fn decode_versioned(json: &str) -> serde_json::Result<VersionedCoreSnapshot> {
    serde_json::from_str(json)
}

fn handle_signal_json(
    json: &str,
    gate: &mut RevisionGate,
    sender: &SnapshotSender,
    passive_intent: &mut PassiveIntent,
    request_repaint: impl FnOnce(),
) -> VersionedHandling {
    let Ok(event) = decode_versioned(json) else {
        eprintln!("OverCrow: malformed Core snapshot signal; reconciling");
        return VersionedHandling::Reconcile;
    };
    apply_versioned(event, gate, sender, passive_intent, request_repaint)
}

fn apply_versioned(
    event: VersionedCoreSnapshot,
    gate: &mut RevisionGate,
    sender: &SnapshotSender,
    passive_intent: &mut PassiveIntent,
    request_repaint: impl FnOnce(),
) -> VersionedHandling {
    match gate.apply(event) {
        RevisionDecision::Apply(event) => {
            passive_intent.record_snapshot(&event.snapshot);
            let passive_confirmed =
                event.snapshot.overlay_mode == overcrow_protocol::OverlayMode::Passive;
            publish_update(
                SnapshotUpdate::confirmed(event.snapshot, passive_confirmed),
                sender,
                request_repaint,
            );
            VersionedHandling::Applied
        }
        RevisionDecision::Ignore => VersionedHandling::Ignored,
        RevisionDecision::Reconcile => VersionedHandling::Reconcile,
    }
}

fn publish_update(update: SnapshotUpdate, sender: &SnapshotSender, request_repaint: impl FnOnce()) {
    if sender.publish(update) {
        request_repaint();
    }
}

#[cfg(test)]
#[path = "core_client_tests.rs"]
mod tests;
