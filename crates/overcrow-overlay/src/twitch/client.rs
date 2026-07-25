use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    pin::Pin,
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures_util::{SinkExt, StreamExt};
use overcrow_config::normalize_twitch_channel;
use overcrow_logging::EventLogger;
use tokio::sync::{
    mpsc::{Receiver as CommandReceiver, Sender as CommandSender},
    watch,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use zeroize::Zeroizing;

use super::{
    auth::{AuthError, AuthMachine, AuthPresentation},
    credentials::{CredentialStore, default_credential_store},
    eventsub::{
        EVENTSUB_MESSAGE_MAX_BYTES, EventSubKind, EventSubParseError, EventSubRevocation,
        parse_eventsub_message, valid_eventsub_url,
    },
    http::{ChatSubscription, HttpError, TwitchHttp, TwitchUser, UreqTwitchHttp},
    model::{
        DeviceAuthorization, MessageBuffer, TWITCH_MESSAGE_MAX_CHARS, TwitchCommand,
        TwitchConnectionState, TwitchFailureCategory, TwitchSendReceipt, TwitchSendReceiptState,
        TwitchSnapshot,
    },
};
use crate::runtime::{
    LatestPublisher, LatestReceiver, VersionedValue, latest_channel,
    widget_diagnostics::{FailureCategory, Provider, ProviderDiagnostics},
};

/// Public Twitch application Client ID for PlayerVox OverCrow.
///
/// Twitch public clients ship this identifier in the binary. It is not a
/// secret and forks may replace it at compile time.
const DEFAULT_TWITCH_CLIENT_ID: &str = "4mnhecoh0nif054yubgwkkj2k7bma2";
const INITIAL_EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const COMMAND_CAPACITY: usize = 32;
const DELIVERY_DEDUP_CAPACITY: usize = 512;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAXIMUM_BACKOFF: Duration = Duration::from_secs(60);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE_GRACE: Duration = Duration::from_secs(10);
const CHAT_PUBLICATION_DELAY: Duration = Duration::from_millis(33);
const WORKER_THREAD_NAME: &str = "overcrow-twitch-provider";

fn configured_twitch_client_id() -> &'static str {
    match option_env!("OVERCROW_TWITCH_CLIENT_ID") {
        Some(id) if !id.is_empty() => id,
        _ => DEFAULT_TWITCH_CLIENT_ID,
    }
}

pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send + 'a>>;
pub type SocketFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    Connection,
    Timeout,
    Protocol,
    Authentication,
    ChannelUnavailable,
    Provider,
}

pub trait EventSocket: Send {
    fn next(&mut self) -> SocketFuture<'_, Option<String>>;
    fn close(&mut self) -> SocketFuture<'_, ()>;
}

pub trait TwitchBackend: Send + 'static {
    fn http(&mut self) -> &mut dyn TwitchHttp;
    fn connect_eventsub(&mut self, url: &str) -> BackendFuture<'_, Box<dyn EventSocket>>;
}

#[derive(Default)]
struct ProductionBackend {
    http: UreqTwitchHttp,
}

impl TwitchBackend for ProductionBackend {
    fn http(&mut self) -> &mut dyn TwitchHttp {
        &mut self.http
    }

    fn connect_eventsub(&mut self, url: &str) -> BackendFuture<'_, Box<dyn EventSocket>> {
        let url = url.to_owned();
        Box::pin(async move {
            if !valid_eventsub_url(&url) {
                return Err(BackendError::Protocol);
            }
            let config = WebSocketConfig::default()
                .read_buffer_size(4 * 1024)
                .write_buffer_size(4 * 1024)
                .max_write_buffer_size(EVENTSUB_MESSAGE_MAX_BYTES)
                .max_message_size(Some(EVENTSUB_MESSAGE_MAX_BYTES))
                .max_frame_size(Some(EVENTSUB_MESSAGE_MAX_BYTES));
            let (stream, _) = connect_async_with_config(url, Some(config), true)
                .await
                .map_err(|_| BackendError::Connection)?;
            Ok(Box::new(TungsteniteEventSocket { stream }) as Box<dyn EventSocket>)
        })
    }
}

type TwitchWebSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct TungsteniteEventSocket {
    stream: TwitchWebSocket,
}

impl EventSocket for TungsteniteEventSocket {
    fn next(&mut self) -> SocketFuture<'_, Option<String>> {
        Box::pin(async move {
            loop {
                match self.stream.next().await {
                    Some(Ok(Message::Text(text))) => return Ok(Some(text.to_string())),
                    Some(Ok(Message::Ping(payload))) => {
                        self.stream
                            .send(Message::Pong(payload))
                            .await
                            .map_err(|_| BackendError::Connection)?;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => return Ok(None),
                    Some(Ok(Message::Binary(_) | Message::Frame(_))) => {
                        return Err(BackendError::Protocol);
                    }
                    Some(Err(_)) => return Err(BackendError::Connection),
                }
            }
        })
    }

    fn close(&mut self) -> SocketFuture<'_, ()> {
        Box::pin(async move {
            self.stream
                .close(None)
                .await
                .map_err(|_| BackendError::Connection)
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TwitchGate {
    pub lifecycle_enabled: bool,
    pub active_game_authorized: bool,
    pub widget_enabled: bool,
    pub channel: Option<String>,
}

impl TwitchGate {
    fn normalized(mut self) -> Self {
        self.channel = self
            .channel
            .as_deref()
            .and_then(|channel| normalize_twitch_channel(channel).ok());
        self
    }

    fn allows_control(&self) -> bool {
        self.lifecycle_enabled && self.active_game_authorized && self.widget_enabled
    }

    fn is_open(&self) -> bool {
        self.allows_control() && self.channel.is_some()
    }
}

#[derive(Clone, Copy)]
pub struct WorkerTiming {
    operation_timeout: Duration,
    initial_backoff: Duration,
    maximum_backoff: Duration,
}

impl Default for WorkerTiming {
    fn default() -> Self {
        Self {
            operation_timeout: OPERATION_TIMEOUT,
            initial_backoff: INITIAL_BACKOFF,
            maximum_backoff: MAXIMUM_BACKOFF,
        }
    }
}

impl WorkerTiming {
    #[cfg(test)]
    pub fn for_tests(operation_timeout: Duration, initial_backoff: Duration) -> Self {
        Self {
            operation_timeout,
            initial_backoff,
            maximum_backoff: operation_timeout.max(initial_backoff),
        }
    }
}

pub struct TwitchClient {
    snapshots: LatestReceiver<TwitchSnapshot>,
    commands: CommandSender<TwitchCommand>,
    gate: watch::Sender<TwitchGate>,
    shutdown: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}

impl TwitchClient {
    pub fn spawn(logger: EventLogger, request_repaint: impl Fn() + Send + Sync + 'static) -> Self {
        Self::spawn_backend(
            ProductionBackend::default(),
            configured_twitch_client_id(),
            default_credential_store(),
            logger,
            request_repaint,
            WorkerTiming::default(),
        )
    }

    #[cfg(test)]
    pub fn spawn_with_backend(
        backend: impl TwitchBackend,
        client_id: &str,
        store: Arc<dyn CredentialStore>,
        request_repaint: impl Fn() + Send + Sync + 'static,
        timing: WorkerTiming,
    ) -> Self {
        Self::spawn_backend(
            backend,
            client_id,
            store,
            EventLogger::disabled(),
            request_repaint,
            timing,
        )
    }

    fn spawn_backend(
        backend: impl TwitchBackend,
        client_id: &str,
        store: Arc<dyn CredentialStore>,
        logger: EventLogger,
        request_repaint: impl Fn() + Send + Sync + 'static,
        timing: WorkerTiming,
    ) -> Self {
        let (publisher, snapshots) = latest_channel(TwitchSnapshot::default());
        let (commands, command_receiver) = command_channel();
        let (gate, gate_receiver) = watch::channel(TwitchGate::default());
        let (shutdown, shutdown_receiver) = watch::channel(false);
        let request_repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(request_repaint);
        let worker_repaint = Arc::clone(&request_repaint);
        let client_id = client_id.to_owned();
        let spawn_diagnostics = ProviderDiagnostics::new(logger.clone(), Provider::Twitch);
        let worker = thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_owned())
            .spawn(move || {
                let mut diagnostics = ProviderDiagnostics::new(logger, Provider::Twitch);
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    diagnostics.failed(FailureCategory::Startup);
                    return;
                };
                runtime.block_on(run_provider(
                    backend,
                    AuthMachine::new(client_id, store),
                    publisher,
                    command_receiver,
                    gate_receiver,
                    shutdown_receiver,
                    worker_repaint,
                    timing,
                    diagnostics,
                ));
            })
            .inspect_err(|_| {
                let mut diagnostics = spawn_diagnostics;
                diagnostics.failed(FailureCategory::Startup);
            })
            .ok();

        Self {
            snapshots,
            commands,
            gate,
            shutdown,
            worker,
        }
    }

    pub fn set_gate(&self, gate: TwitchGate) {
        let gate = gate.normalized();
        if *self.gate.borrow() != gate {
            self.gate.send_replace(gate);
        }
    }

    pub fn try_send(&self, command: TwitchCommand) -> bool {
        self.commands.try_send(command).is_ok()
    }

    pub fn take_latest(&self) -> Option<VersionedValue<TwitchSnapshot>> {
        self.snapshots.take_latest()
    }
}

impl Drop for TwitchClient {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        // All worker operations are bounded. Detaching avoids blocking the UI
        // thread while the shutdown signal reaches an in-flight network call.
        self.worker.take();
    }
}

pub fn command_channel() -> (CommandSender<TwitchCommand>, CommandReceiver<TwitchCommand>) {
    tokio::sync::mpsc::channel(COMMAND_CAPACITY)
}

#[derive(Default)]
struct DeliveryDedup {
    order: VecDeque<String>,
    entries: HashSet<String>,
}

impl DeliveryDedup {
    fn insert(&mut self, id: String) -> bool {
        if self.entries.contains(&id) {
            return false;
        }
        if self.order.len() == DELIVERY_DEDUP_CAPACITY
            && let Some(expired) = self.order.pop_front()
        {
            self.entries.remove(&expired);
        }
        self.entries.insert(id.clone());
        self.order.push_back(id);
        true
    }

    fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }
}

struct WorkerState {
    connection: TwitchConnectionState,
    channel: Option<String>,
    channel_identity: Option<TwitchUser>,
    buffer: MessageBuffer,
    send_receipt: Option<TwitchSendReceipt>,
    generation: u64,
    user_paused: bool,
    delivery_dedup: DeliveryDedup,
    snapshot_due: Option<Instant>,
}

impl Default for WorkerState {
    fn default() -> Self {
        Self {
            connection: TwitchConnectionState::Inert,
            channel: None,
            channel_identity: None,
            buffer: MessageBuffer::default(),
            send_receipt: None,
            generation: 0,
            user_paused: false,
            delivery_dedup: DeliveryDedup::default(),
            snapshot_due: None,
        }
    }
}

impl WorkerState {
    fn snapshot(&self, auth: &AuthMachine) -> TwitchSnapshot {
        let (authorization, connection) = match auth.presentation() {
            AuthPresentation::AwaitingUser {
                user_code,
                verification_uri,
                expires_at,
            } => (
                Some(DeviceAuthorization {
                    user_code: user_code.clone(),
                    verification_uri: verification_uri.clone(),
                    expires_at: *expires_at,
                }),
                TwitchConnectionState::Authorizing,
            ),
            AuthPresentation::Failed(category) => (None, TwitchConnectionState::Failed(*category)),
            AuthPresentation::Disconnected | AuthPresentation::Connected { .. } => {
                (None, self.connection.clone())
            }
        };
        TwitchSnapshot {
            generation: self.generation,
            channel: self.channel.clone(),
            connection,
            messages: self.buffer.snapshot(),
            authorization,
            authenticated_login: auth.authenticated_login().map(str::to_owned),
            credentials_available: auth.credentials_available(),
            credentials_persisted: auth.credentials_persisted(),
            client_configured: auth.is_configured(),
            send_receipt: self.send_receipt.clone(),
        }
    }

    fn clear_private_state(&mut self) {
        self.channel_identity = None;
        self.buffer.clear();
        self.send_receipt = None;
        self.delivery_dedup.clear();
        self.snapshot_due = None;
        self.generation = self.generation.saturating_add(1);
    }

    fn pause_chat(&mut self) {
        self.user_paused = true;
        self.connection = TwitchConnectionState::Disconnected;
    }

    fn schedule_chat_publication(&mut self) {
        if self.snapshot_due.is_none() {
            self.snapshot_due = Instant::now().checked_add(CHAT_PUBLICATION_DELAY);
        }
    }
}

struct ConnectedSession {
    socket: Box<dyn EventSocket>,
    keepalive_timeout: Duration,
    channel: TwitchUser,
}

enum LiveWake {
    Shutdown,
    Gate,
    Command(Option<TwitchCommand>),
    Socket(Result<Option<String>, BackendError>),
    KeepaliveExpired,
    AuthDeadline,
    SnapshotDue,
}

#[allow(clippy::too_many_arguments)]
async fn run_provider(
    mut backend: impl TwitchBackend,
    mut auth: AuthMachine,
    publisher: LatestPublisher<TwitchSnapshot>,
    mut commands: CommandReceiver<TwitchCommand>,
    mut gate_receiver: watch::Receiver<TwitchGate>,
    mut shutdown: watch::Receiver<bool>,
    request_repaint: Arc<dyn Fn() + Send + Sync>,
    timing: WorkerTiming,
    mut diagnostics: ProviderDiagnostics,
) {
    let mut state = WorkerState::default();
    let mut active_gate = TwitchGate::default();
    let mut socket: Option<Box<dyn EventSocket>> = None;
    let mut keepalive_timeout: Option<Duration> = None;
    let mut keepalive_deadline: Option<Instant> = None;
    let mut restore_attempted = false;
    let mut reconnect_at = Instant::now();
    let mut backoff = Backoff::new(timing.initial_backoff, timing.maximum_backoff);

    publish(&publisher, &state, &auth, request_repaint.as_ref());
    loop {
        if *shutdown.borrow() {
            close_socket(&mut socket, timing.operation_timeout).await;
            return;
        }

        let gate = gate_receiver.borrow().clone();
        if gate != active_gate {
            let channel_changed = gate.channel != active_gate.channel;
            let lost_lifecycle = !gate.lifecycle_enabled || !gate.widget_enabled;
            close_socket(&mut socket, timing.operation_timeout).await;
            keepalive_timeout = None;
            keepalive_deadline = None;
            if lost_lifecycle || channel_changed {
                state.clear_private_state();
            }
            if channel_changed {
                state.user_paused = false;
            }
            if lost_lifecycle {
                auth.cancel_pending();
            }
            state.channel = gate.channel.clone();
            state.connection = if gate.allows_control() {
                TwitchConnectionState::Disconnected
            } else {
                TwitchConnectionState::Inert
            };
            active_gate = gate;
            reconnect_at = Instant::now();
            backoff.reset();
            publish(&publisher, &state, &auth, request_repaint.as_ref());
        }

        if !active_gate.allows_control() {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return; }
                }
                changed = gate_receiver.changed() => {
                    if changed.is_err() { return; }
                }
                command = commands.recv() => {
                    let Some(command) = command else { return; };
                    handle_closed_command(command, backend.http(), &mut auth, &mut state, false, &mut diagnostics);
                    publish(&publisher, &state, &auth, request_repaint.as_ref());
                }
            }
            continue;
        }

        if !restore_attempted {
            restore_attempted = true;
            let _ = auth.restore(backend.http(), Instant::now(), wall_secs());
            report_auth_failure(&auth, &mut diagnostics);
            publish(&publisher, &state, &auth, request_repaint.as_ref());
            continue;
        }

        if !auth.can_connect() || !active_gate.is_open() || state.user_paused {
            state.connection = if state.user_paused || active_gate.channel.is_none() {
                TwitchConnectionState::Disconnected
            } else {
                state.connection.clone()
            };
            let auth_deadline = auth_deadline(&auth);
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return; }
                }
                changed = gate_receiver.changed() => {
                    if changed.is_err() { return; }
                }
                command = commands.recv() => {
                    let Some(command) = command else { return; };
                    handle_closed_command(command, backend.http(), &mut auth, &mut state, true, &mut diagnostics);
                }
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(auth_deadline)) => {
                    let _ = auth.tick(backend.http(), Instant::now(), wall_secs());
                    report_auth_failure(&auth, &mut diagnostics);
                }
            }
            publish(&publisher, &state, &auth, request_repaint.as_ref());
            continue;
        }

        if socket.is_none() {
            let now = Instant::now();
            if now < reconnect_at {
                let auth_deadline = auth_deadline(&auth);
                let deadline = reconnect_at.min(auth_deadline);
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { return; }
                    }
                    changed = gate_receiver.changed() => {
                        if changed.is_err() { return; }
                    }
                    command = commands.recv() => {
                        let Some(command) = command else { return; };
                        handle_closed_command(command, backend.http(), &mut auth, &mut state, true, &mut diagnostics);
                    }
                    () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                        if deadline == auth_deadline {
                            let _ = auth.tick(backend.http(), Instant::now(), wall_secs());
                            report_auth_failure(&auth, &mut diagnostics);
                        }
                    }
                }
                publish(&publisher, &state, &auth, request_repaint.as_ref());
                continue;
            }

            state.connection = TwitchConnectionState::Connecting;
            publish(&publisher, &state, &auth, request_repaint.as_ref());
            match connect_initial(&mut backend, &auth, &active_gate, timing.operation_timeout).await
            {
                Ok(session) => {
                    state.channel_identity = Some(session.channel);
                    socket = Some(session.socket);
                    keepalive_timeout = Some(session.keepalive_timeout);
                    keepalive_deadline = keepalive_deadline_after(session.keepalive_timeout);
                    state.connection = TwitchConnectionState::Joined;
                    backoff.reset();
                    diagnostics.recovered();
                }
                Err(error) => {
                    apply_connection_failure(error, &mut auth, &mut state);
                    diagnostics.failed(error.failure_category());
                    reconnect_at = Instant::now()
                        .checked_add(backoff.next_delay())
                        .unwrap_or_else(Instant::now);
                }
            }
            publish(&publisher, &state, &auth, request_repaint.as_ref());
            continue;
        }

        let auth_deadline = auth_deadline(&auth);
        let keepalive = keepalive_deadline.unwrap_or_else(far_future);
        let snapshot_due = state.snapshot_due.unwrap_or_else(far_future);
        let wake = {
            let Some(current) = socket.as_mut() else {
                continue;
            };
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        LiveWake::Shutdown
                    } else {
                        continue;
                    }
                }
                changed = gate_receiver.changed() => {
                    if changed.is_err() { LiveWake::Shutdown } else { LiveWake::Gate }
                }
                command = commands.recv() => LiveWake::Command(command),
                message = current.next() => LiveWake::Socket(message),
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(keepalive)) => {
                    LiveWake::KeepaliveExpired
                }
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(auth_deadline)) => {
                    LiveWake::AuthDeadline
                }
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(snapshot_due)) => {
                    LiveWake::SnapshotDue
                }
            }
        };

        let mut publish_now = true;
        match wake {
            LiveWake::Shutdown => {
                close_socket(&mut socket, timing.operation_timeout).await;
                return;
            }
            LiveWake::Gate => continue,
            LiveWake::Command(None) => return,
            LiveWake::Command(Some(command)) => {
                let disconnect = handle_live_command(
                    command,
                    &mut backend,
                    &mut auth,
                    &mut state,
                    &active_gate,
                    &mut diagnostics,
                );
                if disconnect {
                    close_socket(&mut socket, timing.operation_timeout).await;
                    keepalive_timeout = None;
                    keepalive_deadline = None;
                    reconnect_at = Instant::now();
                }
            }
            LiveWake::Socket(Ok(Some(raw))) => {
                keepalive_deadline = keepalive_timeout.and_then(keepalive_deadline_after);
                let parsed = match parse_eventsub_message(&raw) {
                    Ok(message) => message,
                    Err(error) => {
                        diagnostics.failed(match error {
                            EventSubParseError::Oversized => FailureCategory::Validation,
                            EventSubParseError::Malformed => FailureCategory::Parse,
                        });
                        continue;
                    }
                };
                if state.delivery_dedup.insert(parsed.delivery_id) {
                    match parsed.kind {
                        EventSubKind::Reconnect { url } => {
                            state.connection = TwitchConnectionState::Reconnecting;
                            publish(&publisher, &state, &auth, request_repaint.as_ref());
                            match connect_replacement(&mut backend, &url, timing.operation_timeout)
                                .await
                            {
                                Ok((replacement, timeout)) => {
                                    close_socket(&mut socket, timing.operation_timeout).await;
                                    socket = Some(replacement);
                                    keepalive_timeout = Some(timeout);
                                    keepalive_deadline = keepalive_deadline_after(timeout);
                                    state.connection = TwitchConnectionState::Joined;
                                    diagnostics.recovered();
                                }
                                Err(error) => {
                                    close_socket(&mut socket, timing.operation_timeout).await;
                                    keepalive_timeout = None;
                                    apply_connection_failure(error, &mut auth, &mut state);
                                    diagnostics.failed(error.failure_category());
                                    reconnect_at = reconnect_after(&mut backoff);
                                }
                            }
                        }
                        kind => match apply_event(kind, &mut state) {
                            Ok(true) => {
                                diagnostics.recovered();
                                state.schedule_chat_publication();
                                publish_now = false;
                            }
                            Ok(false) => {
                                diagnostics.recovered();
                                publish_now = false;
                            }
                            Err(BackendError::Protocol) => {
                                diagnostics.failed(FailureCategory::Validation);
                                publish_now = false;
                            }
                            Err(error) => {
                                close_socket(&mut socket, timing.operation_timeout).await;
                                keepalive_timeout = None;
                                apply_connection_failure(error, &mut auth, &mut state);
                                diagnostics.failed(error.failure_category());
                                reconnect_at = reconnect_after(&mut backoff);
                            }
                        },
                    }
                } else {
                    publish_now = false;
                }
            }
            LiveWake::Socket(Ok(None) | Err(_)) | LiveWake::KeepaliveExpired => {
                close_socket(&mut socket, timing.operation_timeout).await;
                keepalive_timeout = None;
                keepalive_deadline = None;
                state.connection = TwitchConnectionState::Reconnecting;
                diagnostics.failed(FailureCategory::Connection);
                reconnect_at = reconnect_after(&mut backoff);
            }
            LiveWake::AuthDeadline => {
                let _ = auth.tick(backend.http(), Instant::now(), wall_secs());
                report_auth_failure(&auth, &mut diagnostics);
                if !auth.can_connect() {
                    close_socket(&mut socket, timing.operation_timeout).await;
                    keepalive_timeout = None;
                    keepalive_deadline = None;
                    state.connection = TwitchConnectionState::Disconnected;
                }
            }
            LiveWake::SnapshotDue => {
                state.snapshot_due = None;
            }
        }
        if publish_now {
            state.snapshot_due = None;
            publish(&publisher, &state, &auth, request_repaint.as_ref());
        }
    }
}

async fn connect_initial(
    backend: &mut impl TwitchBackend,
    auth: &AuthMachine,
    gate: &TwitchGate,
    timeout: Duration,
) -> Result<ConnectedSession, BackendError> {
    let client_id = auth.client_id().to_owned();
    let access_token = Zeroizing::new(
        auth.access_token()
            .ok_or(BackendError::Authentication)?
            .to_owned(),
    );
    let user_id = auth
        .authenticated_user_id()
        .ok_or(BackendError::Authentication)?
        .to_owned();
    let channel_login = gate
        .channel
        .as_deref()
        .ok_or(BackendError::ChannelUnavailable)?;
    let channel = backend
        .http()
        .resolve_channel(&client_id, &access_token, channel_login)
        .map_err(|error| map_http_error(error, true))?;
    let (socket, session_id, keepalive_timeout) =
        connect_and_welcome(backend, INITIAL_EVENTSUB_URL, timeout).await?;
    for subscription in [
        ChatSubscription::Message,
        ChatSubscription::Clear,
        ChatSubscription::MessageDelete,
    ] {
        backend
            .http()
            .create_chat_subscription(
                &client_id,
                &access_token,
                &session_id,
                &channel.id,
                &user_id,
                subscription,
            )
            .map_err(|error| map_http_error(error, false))?;
    }
    Ok(ConnectedSession {
        socket,
        keepalive_timeout,
        channel,
    })
}

async fn connect_replacement(
    backend: &mut impl TwitchBackend,
    url: &str,
    timeout: Duration,
) -> Result<(Box<dyn EventSocket>, Duration), BackendError> {
    let (socket, _, keepalive) = connect_and_welcome(backend, url, timeout).await?;
    Ok((socket, keepalive))
}

async fn connect_and_welcome(
    backend: &mut impl TwitchBackend,
    url: &str,
    timeout: Duration,
) -> Result<(Box<dyn EventSocket>, String, Duration), BackendError> {
    if !valid_eventsub_url(url) {
        return Err(BackendError::Protocol);
    }
    let mut socket = tokio::time::timeout(timeout, backend.connect_eventsub(url))
        .await
        .map_err(|_| BackendError::Timeout)??;
    let raw = tokio::time::timeout(timeout, socket.next())
        .await
        .map_err(|_| BackendError::Timeout)??
        .ok_or(BackendError::Connection)?;
    let message = parse_eventsub_message(&raw).map_err(|_| BackendError::Protocol)?;
    match message.kind {
        EventSubKind::Welcome {
            session_id,
            keepalive_timeout,
        } => Ok((socket, session_id, keepalive_timeout)),
        _ => Err(BackendError::Protocol),
    }
}

fn apply_event(event: EventSubKind, state: &mut WorkerState) -> Result<bool, BackendError> {
    let expected_broadcaster = state
        .channel_identity
        .as_ref()
        .map(|channel| channel.id.as_str())
        .ok_or(BackendError::Protocol)?;
    match event {
        EventSubKind::Keepalive | EventSubKind::Unsupported => Ok(false),
        EventSubKind::ChatMessage {
            broadcaster_user_id,
            message,
        } => {
            if broadcaster_user_id != expected_broadcaster {
                return Err(BackendError::Protocol);
            }
            state.buffer.upsert_received(message, Instant::now());
            Ok(true)
        }
        EventSubKind::ChatClear {
            broadcaster_user_id,
        } => {
            if broadcaster_user_id != expected_broadcaster {
                return Err(BackendError::Protocol);
            }
            state.buffer.clear();
            Ok(true)
        }
        EventSubKind::ChatMessageDelete {
            broadcaster_user_id,
            message_id,
        } => {
            if broadcaster_user_id != expected_broadcaster {
                return Err(BackendError::Protocol);
            }
            state.buffer.remove_message(&message_id);
            Ok(true)
        }
        EventSubKind::Revocation(EventSubRevocation::Authentication) => {
            Err(BackendError::Authentication)
        }
        EventSubKind::Revocation(EventSubRevocation::ChannelUnavailable) => {
            Err(BackendError::ChannelUnavailable)
        }
        EventSubKind::Revocation(EventSubRevocation::Provider) => Err(BackendError::Provider),
        EventSubKind::Welcome { .. } | EventSubKind::Reconnect { .. } => {
            Err(BackendError::Protocol)
        }
    }
}

fn handle_closed_command(
    command: TwitchCommand,
    http: &mut dyn TwitchHttp,
    auth: &mut AuthMachine,
    state: &mut WorkerState,
    allow_auth: bool,
    diagnostics: &mut ProviderDiagnostics,
) {
    let result = match command {
        TwitchCommand::BeginAuthentication if allow_auth => {
            if auth.can_connect() {
                state.user_paused = false;
                Ok(())
            } else {
                auth.begin(http, Instant::now())
            }
        }
        TwitchCommand::CancelAuthentication => {
            auth.cancel();
            Ok(())
        }
        TwitchCommand::Disconnect => {
            state.pause_chat();
            Ok(())
        }
        TwitchCommand::SignOut => {
            let result = auth.disconnect(http);
            state.user_paused = false;
            state.clear_private_state();
            result
        }
        TwitchCommand::Reconnect if allow_auth => {
            state.user_paused = false;
            Ok(())
        }
        TwitchCommand::SendMessage { request_id, .. } => {
            state.send_receipt = Some(TwitchSendReceipt {
                request_id,
                state: TwitchSendReceiptState::Rejected,
            });
            Ok(())
        }
        TwitchCommand::BeginAuthentication | TwitchCommand::Reconnect => Ok(()),
    };
    if let Err(error) = result {
        diagnostics.failed(auth_error_category(error));
    }
}

fn handle_live_command(
    command: TwitchCommand,
    backend: &mut impl TwitchBackend,
    auth: &mut AuthMachine,
    state: &mut WorkerState,
    gate: &TwitchGate,
    diagnostics: &mut ProviderDiagnostics,
) -> bool {
    match command {
        TwitchCommand::SendMessage {
            request_id,
            generation,
            channel,
            text,
            reply_to,
        } => send_message(
            request_id,
            generation,
            &channel,
            &text,
            reply_to.as_deref(),
            backend.http(),
            auth,
            state,
            gate,
            diagnostics,
        ),
        TwitchCommand::Reconnect => {
            state.user_paused = false;
            state.connection = TwitchConnectionState::Reconnecting;
            true
        }
        TwitchCommand::Disconnect => {
            state.pause_chat();
            true
        }
        TwitchCommand::SignOut => {
            if let Err(error) = auth.disconnect(backend.http()) {
                diagnostics.failed(auth_error_category(error));
            }
            state.user_paused = false;
            state.clear_private_state();
            state.connection = TwitchConnectionState::Disconnected;
            true
        }
        TwitchCommand::BeginAuthentication => false,
        TwitchCommand::CancelAuthentication => {
            auth.cancel();
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn send_message(
    request_id: u64,
    generation: u64,
    channel: &str,
    text: &str,
    reply_to: Option<&str>,
    http: &mut dyn TwitchHttp,
    auth: &mut AuthMachine,
    state: &mut WorkerState,
    gate: &TwitchGate,
    diagnostics: &mut ProviderDiagnostics,
) -> bool {
    let reject = |state: &mut WorkerState| {
        state.send_receipt = Some(TwitchSendReceipt {
            request_id,
            state: TwitchSendReceiptState::Rejected,
        });
    };
    let normalized = normalize_twitch_channel(channel).ok();
    if state.connection != TwitchConnectionState::Joined
        || generation != state.generation
        || normalized.as_deref() != gate.channel.as_deref()
        || text.trim().is_empty()
        || text.chars().count() > TWITCH_MESSAGE_MAX_CHARS
        || text.chars().any(char::is_control)
    {
        reject(state);
        return false;
    }
    let Some(channel_identity) = state.channel_identity.as_ref() else {
        reject(state);
        return false;
    };
    let Some(access_token) = auth.access_token() else {
        reject(state);
        return false;
    };
    let Some(sender_id) = auth.authenticated_user_id() else {
        reject(state);
        return false;
    };
    let display_name = auth.authenticated_login().unwrap_or_default().to_owned();
    // The request ID is already unique among this worker's bounded pending
    // sends. It is local-only, so random UUID generation adds no value.
    let nonce = request_id.to_string();
    if !state
        .buffer
        .push_pending(nonce.clone(), display_name, text.to_owned(), Instant::now())
    {
        reject(state);
        return false;
    }

    let result = http.send_chat_message(
        auth.client_id(),
        access_token,
        &channel_identity.id,
        sender_id,
        text,
        reply_to,
    );
    match result {
        Ok(result) if result.is_sent => {
            if let Some(message_id) = result.message_id {
                state.buffer.mark_sent(&nonce, message_id);
                state.send_receipt = Some(TwitchSendReceipt {
                    request_id,
                    state: TwitchSendReceiptState::Accepted,
                });
                diagnostics.recovered();
                false
            } else {
                state.buffer.fail_pending(&nonce);
                reject(state);
                diagnostics.failed(FailureCategory::Response);
                false
            }
        }
        Ok(_) => {
            state.buffer.fail_pending(&nonce);
            reject(state);
            diagnostics.failed(FailureCategory::Response);
            false
        }
        Err(error) => {
            state.buffer.fail_pending(&nonce);
            reject(state);
            let error = map_http_error(error, false);
            diagnostics.failed(error.failure_category());
            if error == BackendError::Authentication {
                apply_connection_failure(error, auth, state);
                true
            } else {
                false
            }
        }
    }
}

async fn close_socket(socket: &mut Option<Box<dyn EventSocket>>, timeout: Duration) {
    if let Some(mut current) = socket.take() {
        let _ = tokio::time::timeout(timeout, current.close()).await;
    }
}

fn apply_connection_failure(error: BackendError, auth: &mut AuthMachine, state: &mut WorkerState) {
    state.channel_identity = None;
    state.connection = match error {
        BackendError::Authentication => {
            auth.invalidate(TwitchFailureCategory::Authentication);
            TwitchConnectionState::Failed(TwitchFailureCategory::Authentication)
        }
        BackendError::ChannelUnavailable => {
            TwitchConnectionState::Failed(TwitchFailureCategory::ChannelUnavailable)
        }
        BackendError::Connection | BackendError::Timeout => TwitchConnectionState::Reconnecting,
        BackendError::Protocol | BackendError::Provider => {
            TwitchConnectionState::Failed(TwitchFailureCategory::ProviderResponse)
        }
    };
}

fn map_http_error(error: HttpError, resolving_channel: bool) -> BackendError {
    match error {
        HttpError::Timeout => BackendError::Timeout,
        HttpError::Transport => BackendError::Connection,
        HttpError::Status(401 | 403) => BackendError::Authentication,
        HttpError::Status(404) if resolving_channel => BackendError::ChannelUnavailable,
        HttpError::InvalidUrl | HttpError::InvalidRequest | HttpError::BodyTooLarge => {
            BackendError::Protocol
        }
        HttpError::Status(_) | HttpError::ProviderResponse => {
            if resolving_channel {
                BackendError::ChannelUnavailable
            } else {
                BackendError::Provider
            }
        }
    }
}

fn report_auth_failure(auth: &AuthMachine, diagnostics: &mut ProviderDiagnostics) {
    let category = match auth.presentation() {
        AuthPresentation::Failed(TwitchFailureCategory::CredentialStore) => {
            Some(FailureCategory::Filesystem)
        }
        AuthPresentation::Failed(
            TwitchFailureCategory::Authentication | TwitchFailureCategory::AuthorizationExpired,
        ) => Some(FailureCategory::Response),
        AuthPresentation::Failed(_) => Some(FailureCategory::Validation),
        _ => None,
    };
    if let Some(category) = category {
        diagnostics.failed(category);
    }
}

fn auth_error_category(error: AuthError) -> FailureCategory {
    match error {
        AuthError::ClientUnavailable | AuthError::SessionExists => FailureCategory::Validation,
        AuthError::InvalidCredentials => FailureCategory::Response,
        AuthError::Credentials(_) => FailureCategory::Filesystem,
        AuthError::Provider(HttpError::Timeout) => FailureCategory::Timeout,
        AuthError::Provider(HttpError::Transport) => FailureCategory::Transport,
        AuthError::Provider(HttpError::InvalidUrl | HttpError::InvalidRequest) => {
            FailureCategory::Validation
        }
        AuthError::Provider(
            HttpError::Status(_) | HttpError::BodyTooLarge | HttpError::ProviderResponse,
        ) => FailureCategory::Response,
    }
}

fn publish(
    publisher: &LatestPublisher<TwitchSnapshot>,
    state: &WorkerState,
    auth: &AuthMachine,
    request_repaint: &dyn Fn(),
) {
    if publisher.publish(state.snapshot(auth)) {
        request_repaint();
    }
}

fn wall_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn auth_deadline(auth: &AuthMachine) -> Instant {
    auth.next_deadline(Instant::now(), wall_secs())
        .unwrap_or_else(far_future)
}

fn far_future() -> Instant {
    Instant::now()
        .checked_add(Duration::from_secs(24 * 60 * 60))
        .unwrap_or_else(Instant::now)
}

fn keepalive_deadline_after(timeout: Duration) -> Option<Instant> {
    Instant::now().checked_add(timeout.saturating_add(KEEPALIVE_GRACE))
}

fn reconnect_after(backoff: &mut Backoff) -> Instant {
    Instant::now()
        .checked_add(backoff.next_delay())
        .unwrap_or_else(Instant::now)
}

impl BackendError {
    fn failure_category(self) -> FailureCategory {
        match self {
            Self::Connection => FailureCategory::Connection,
            Self::Timeout => FailureCategory::Timeout,
            Self::Protocol => FailureCategory::Validation,
            Self::Authentication | Self::ChannelUnavailable | Self::Provider => {
                FailureCategory::Response
            }
        }
    }
}

struct Backoff {
    initial: Duration,
    current: Duration,
    maximum: Duration,
}

impl Backoff {
    fn new(initial: Duration, maximum: Duration) -> Self {
        Self {
            initial,
            current: initial,
            maximum,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = self.current.saturating_mul(2).min(self.maximum);
        delay
    }

    fn reset(&mut self) {
        self.current = self.initial;
    }
}
