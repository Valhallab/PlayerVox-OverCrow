use std::{
    collections::{HashMap, HashSet},
    env,
    future::Future,
    io::ErrorKind,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use overcrow_logging::EventLogger;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::UnixStream,
    sync::{mpsc, watch},
    time::Instant as TokioInstant,
};

use super::{
    auth::{AuthError, DiscordAuth},
    credentials::{CredentialStore, default_credential_store},
    model::{
        DiscordRpcEvent, VoiceChannel, VoiceParticipant, VoiceSubscriptionEvent, sort_participants,
    },
    oauth::{DiscordOauth, OauthError, UreqDiscordOauth},
    rpc::{DISCORD_RPC_MESSAGE_MAX_BYTES, RpcParseError, parse_rpc_message},
};
use crate::runtime::{
    LatestPublisher, LatestReceiver, VersionedValue, latest_channel,
    widget_diagnostics::{FailureCategory, Provider, ProviderDiagnostics},
};

const FIRST_RPC_SLOT: u8 = 0;
const LAST_RPC_SLOT: u8 = 9;
const RPC_VERSION: u8 = 1;
const IPC_OPCODE_HANDSHAKE: u32 = 0;
const IPC_OPCODE_FRAME: u32 = 1;
const IPC_OPCODE_CLOSE: u32 = 2;
const IPC_OPCODE_PING: u32 = 3;
const IPC_OPCODE_PONG: u32 = 4;
const IPC_HEADER_BYTES: usize = 8;
const RPC_ROOT_MAX: usize = 5;
const COMMAND_CAPACITY: usize = 8;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAXIMUM_BACKOFF: Duration = Duration::from_secs(10);
const WORKER_THREAD_NAME: &str = "overcrow-discord-provider";
const RPC_INVALID_TOKEN: i64 = 4009;
const OFFICIAL_DISCORD_CLIENT_ID: &str = "1533203091757858936";

pub(super) fn configured_discord_client_id() -> &'static str {
    option_env!("OVERCROW_DISCORD_CLIENT_ID").unwrap_or(OFFICIAL_DISCORD_CLIENT_ID)
}

pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send + 'a>>;
pub type SocketFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    Connection,
    Timeout,
    Protocol,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectAttempt {
    pub path: PathBuf,
    pub client_id: String,
}

pub trait RpcSocket: Send {
    fn next(&mut self) -> SocketFuture<'_, Option<String>>;
    fn send(&mut self, message: String) -> SocketFuture<'_, ()>;
    fn close(&mut self) -> SocketFuture<'_, ()>;
}

pub trait DiscordBackend: Send + 'static {
    fn connect(&mut self, attempt: ConnectAttempt) -> BackendFuture<'_, Box<dyn RpcSocket>>;
}

pub(super) struct ProductionBackend;

impl DiscordBackend for ProductionBackend {
    fn connect(&mut self, attempt: ConnectAttempt) -> BackendFuture<'_, Box<dyn RpcSocket>> {
        Box::pin(async move {
            if !valid_client_id(&attempt.client_id) || !is_owned_unix_socket(&attempt.path) {
                return Err(BackendError::Protocol);
            }
            let stream = UnixStream::connect(&attempt.path)
                .await
                .map_err(|_| BackendError::Connection)?;
            let peer = stream.peer_cred().map_err(|_| BackendError::Protocol)?;
            if !peer_uid_is_current(peer.uid()) {
                return Err(BackendError::Protocol);
            }
            let mut socket = DiscordIpcSocket { stream };
            let handshake = serde_json::to_vec(&json!({
                "v": RPC_VERSION,
                "client_id": attempt.client_id,
            }))
            .map_err(|_| BackendError::Protocol)?;
            write_ipc_packet(&mut socket.stream, IPC_OPCODE_HANDSHAKE, &handshake).await?;
            Ok(Box::new(socket) as Box<dyn RpcSocket>)
        })
    }
}

pub(super) struct DiscordIpcSocket<S> {
    pub(super) stream: S,
}

impl<S> RpcSocket for DiscordIpcSocket<S>
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    fn next(&mut self) -> SocketFuture<'_, Option<String>> {
        Box::pin(async move {
            loop {
                let Some((opcode, payload)) = read_ipc_packet(&mut self.stream).await? else {
                    return Ok(None);
                };
                match opcode {
                    IPC_OPCODE_FRAME => {
                        return String::from_utf8(payload)
                            .map(Some)
                            .map_err(|_| BackendError::Protocol);
                    }
                    IPC_OPCODE_PING => {
                        write_ipc_packet(&mut self.stream, IPC_OPCODE_PONG, &payload).await?;
                    }
                    IPC_OPCODE_PONG => {}
                    IPC_OPCODE_CLOSE => return Ok(None),
                    _ => return Err(BackendError::Protocol),
                }
            }
        })
    }

    fn send(&mut self, message: String) -> SocketFuture<'_, ()> {
        Box::pin(async move {
            write_ipc_packet(&mut self.stream, IPC_OPCODE_FRAME, message.as_bytes()).await
        })
    }

    fn close(&mut self) -> SocketFuture<'_, ()> {
        Box::pin(async move {
            self.stream
                .shutdown()
                .await
                .map_err(|_| BackendError::Connection)
        })
    }
}

pub(super) async fn read_ipc_packet(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<Option<(u32, Vec<u8>)>, BackendError> {
    let mut header = [0_u8; IPC_HEADER_BYTES];
    match stream.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Ok(None),
        Err(_) => return Err(BackendError::Connection),
    }
    let opcode = u32::from_le_bytes(header[..4].try_into().map_err(|_| BackendError::Protocol)?);
    let length = u32::from_le_bytes(header[4..].try_into().map_err(|_| BackendError::Protocol)?);
    let length = usize::try_from(length).map_err(|_| BackendError::Protocol)?;
    if length > DISCORD_RPC_MESSAGE_MAX_BYTES {
        return Err(BackendError::Protocol);
    }
    let mut payload = vec![0_u8; length];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|_| BackendError::Connection)?;
    Ok(Some((opcode, payload)))
}

pub(super) async fn write_ipc_packet(
    stream: &mut (impl AsyncWrite + Unpin),
    opcode: u32,
    payload: &[u8],
) -> Result<(), BackendError> {
    let length = u32::try_from(payload.len()).map_err(|_| BackendError::Protocol)?;
    if payload.len() > DISCORD_RPC_MESSAGE_MAX_BYTES {
        return Err(BackendError::Protocol);
    }
    let mut header = [0_u8; IPC_HEADER_BYTES];
    header[..4].copy_from_slice(&opcode.to_le_bytes());
    header[4..].copy_from_slice(&length.to_le_bytes());
    stream
        .write_all(&header)
        .await
        .map_err(|_| BackendError::Connection)?;
    stream
        .write_all(payload)
        .await
        .map_err(|_| BackendError::Connection)?;
    stream.flush().await.map_err(|_| BackendError::Connection)
}

fn rpc_runtime_roots() -> Vec<PathBuf> {
    ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .into_iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .chain(std::iter::once(PathBuf::from("/tmp")))
        .collect()
}

pub(super) fn rpc_socket_candidates(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    roots
        .iter()
        .filter(|root| root.is_absolute())
        .filter(|root| seen.insert((*root).clone()))
        .take(RPC_ROOT_MAX)
        .flat_map(|root| {
            (FIRST_RPC_SLOT..=LAST_RPC_SLOT)
                .map(move |slot| root.join(format!("discord-ipc-{slot}")))
        })
        .collect()
}

pub(super) fn is_owned_unix_socket(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_socket() && metadata.uid() == effective_uid())
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and does not retain pointers.
    unsafe { libc::geteuid() }
}

pub(super) fn peer_uid_is_current(uid: u32) -> bool {
    uid == effective_uid()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscordGate {
    pub lifecycle_enabled: bool,
    pub active_game_authorized: bool,
    pub widget_enabled: bool,
}

impl DiscordGate {
    fn is_open(&self) -> bool {
        self.lifecycle_enabled && self.active_game_authorized && self.widget_enabled
    }

    fn keeps_socket_open(&self, connection: DiscordConnectionState) -> bool {
        self.is_open()
            || (self.lifecycle_enabled
                && self.widget_enabled
                && matches!(
                    connection,
                    DiscordConnectionState::Authorizing | DiscordConnectionState::Authenticating
                ))
    }

    fn retains_display_on_close(&self) -> bool {
        self.lifecycle_enabled && self.widget_enabled
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiscordConnectionState {
    #[default]
    Inert,
    ClientNotConfigured,
    Connecting,
    AuthorizationRequired,
    Authorizing,
    Authenticating,
    Ready,
    DiscordUnavailable,
    Failed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscordSnapshot {
    pub generation: u64,
    pub connection: DiscordConnectionState,
    pub channel: Option<VoiceChannel>,
    pub credentials_available: bool,
    pub credentials_persisted: bool,
    pub client_configured: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscordCommand {
    Connect,
    SignOut,
}

#[derive(Clone, Copy)]
pub struct WorkerTiming {
    connect_timeout: Duration,
    operation_timeout: Duration,
    authorization_timeout: Duration,
    initial_backoff: Duration,
    maximum_backoff: Duration,
}

impl Default for WorkerTiming {
    fn default() -> Self {
        Self {
            connect_timeout: CONNECT_TIMEOUT,
            operation_timeout: OPERATION_TIMEOUT,
            authorization_timeout: AUTHORIZATION_TIMEOUT,
            initial_backoff: INITIAL_BACKOFF,
            maximum_backoff: MAXIMUM_BACKOFF,
        }
    }
}

impl WorkerTiming {
    #[cfg(test)]
    pub fn for_tests(operation_timeout: Duration, initial_backoff: Duration) -> Self {
        Self {
            connect_timeout: operation_timeout,
            operation_timeout,
            authorization_timeout: operation_timeout.saturating_mul(4),
            initial_backoff,
            maximum_backoff: operation_timeout.max(initial_backoff),
        }
    }

    #[cfg(test)]
    pub fn for_tests_with_authorization_timeout(
        operation_timeout: Duration,
        authorization_timeout: Duration,
        initial_backoff: Duration,
    ) -> Self {
        Self {
            connect_timeout: operation_timeout,
            operation_timeout,
            authorization_timeout,
            initial_backoff,
            maximum_backoff: operation_timeout.max(initial_backoff),
        }
    }
}

pub struct DiscordClient {
    snapshots: LatestReceiver<DiscordSnapshot>,
    commands: mpsc::Sender<DiscordCommand>,
    gate: watch::Sender<DiscordGate>,
    shutdown: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}

impl DiscordClient {
    pub fn spawn(logger: EventLogger, request_repaint: impl Fn() + Send + Sync + 'static) -> Self {
        Self::spawn_backend(
            ProductionBackend,
            UreqDiscordOauth::default(),
            configured_discord_client_id(),
            rpc_socket_candidates(&rpc_runtime_roots()),
            default_credential_store(),
            logger,
            request_repaint,
            WorkerTiming::default(),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_backend_and_logger(
        backend: impl DiscordBackend,
        oauth: impl DiscordOauth + 'static,
        client_id: &str,
        rpc_paths: Vec<PathBuf>,
        store: Arc<dyn CredentialStore>,
        logger: EventLogger,
        request_repaint: impl Fn() + Send + Sync + 'static,
        timing: WorkerTiming,
    ) -> Self {
        Self::spawn_backend(
            backend,
            oauth,
            client_id,
            rpc_paths,
            store,
            logger,
            request_repaint,
            timing,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_backend(
        backend: impl DiscordBackend,
        oauth: impl DiscordOauth + 'static,
        client_id: &str,
        rpc_paths: Vec<PathBuf>,
        store: Arc<dyn CredentialStore>,
        logger: EventLogger,
        request_repaint: impl Fn() + Send + Sync + 'static,
        timing: WorkerTiming,
    ) -> Self {
        let client_configured = valid_client_id(client_id);
        let initial = DiscordSnapshot {
            client_configured,
            ..DiscordSnapshot::default()
        };
        let (publisher, snapshots) = latest_channel(initial);
        let (commands, command_receiver) = mpsc::channel(COMMAND_CAPACITY);
        let (gate, gate_receiver) = watch::channel(DiscordGate::default());
        let (shutdown, shutdown_receiver) = watch::channel(false);
        let request_repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(request_repaint);
        let worker_repaint = Arc::clone(&request_repaint);
        let client_id = client_id.to_owned();
        let spawn_diagnostics = ProviderDiagnostics::new(logger.clone(), Provider::Discord);
        let worker = thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_owned())
            .spawn(move || {
                let mut diagnostics = ProviderDiagnostics::new(logger, Provider::Discord);
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    diagnostics.failed(FailureCategory::Startup);
                    return;
                };
                runtime.block_on(run_provider(
                    backend,
                    oauth,
                    DiscordAuth::new(store),
                    client_id,
                    rpc_paths,
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

    pub fn set_gate(&self, gate: DiscordGate) {
        if *self.gate.borrow() != gate {
            self.gate.send_replace(gate);
        }
    }

    pub fn try_send(&self, command: DiscordCommand) -> bool {
        self.commands.try_send(command).is_ok()
    }

    pub fn take_latest(&self) -> Option<VersionedValue<DiscordSnapshot>> {
        self.snapshots.take_latest()
    }
}

impl Drop for DiscordClient {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        // Every operation is bounded. Detaching keeps the egui thread responsive.
        self.worker.take();
    }
}

struct WorkerState {
    snapshot: DiscordSnapshot,
    local_user_id: Option<String>,
    nonce: u64,
    auth_loaded: bool,
    authentication_refresh_attempted: bool,
    pending_authorization_nonce: Option<String>,
    pending_authentication_nonce: Option<String>,
    subscribed_channel_id: Option<String>,
    pending_subscriptions: HashMap<String, PendingSubscription>,
    pending_channel_snapshot_nonce: Option<String>,
    selected_channel_id: Option<Option<String>>,
    channel_snapshot_received: bool,
    session_healthy: bool,
    setup_deadline: Option<TokioInstant>,
}

impl WorkerState {
    fn new(client_configured: bool) -> Self {
        Self {
            snapshot: DiscordSnapshot {
                client_configured,
                ..DiscordSnapshot::default()
            },
            local_user_id: None,
            nonce: 0,
            auth_loaded: false,
            authentication_refresh_attempted: false,
            pending_authorization_nonce: None,
            pending_authentication_nonce: None,
            subscribed_channel_id: None,
            pending_subscriptions: HashMap::new(),
            pending_channel_snapshot_nonce: None,
            selected_channel_id: None,
            channel_snapshot_received: false,
            session_healthy: false,
            setup_deadline: None,
        }
    }

    fn clear_private_state(&mut self) {
        self.snapshot.channel = None;
        self.reset_rpc_session();
    }

    fn suspend_for_gate(&mut self, gate: &DiscordGate) {
        if !gate.retains_display_on_close() {
            self.snapshot.channel = None;
        }
        self.reset_rpc_session();
        self.snapshot.connection = DiscordConnectionState::Inert;
    }

    fn reset_rpc_session(&mut self) {
        self.local_user_id = None;
        self.pending_authorization_nonce = None;
        self.pending_authentication_nonce = None;
        self.subscribed_channel_id = None;
        self.pending_subscriptions.clear();
        self.pending_channel_snapshot_nonce = None;
        self.selected_channel_id = None;
        self.channel_snapshot_received = false;
        self.session_healthy = false;
        self.setup_deadline = None;
    }

    fn next_nonce(&mut self) -> String {
        self.nonce = self.nonce.wrapping_add(1);
        self.nonce.to_string()
    }

    fn arm_setup_deadline(&mut self, timeout: Duration) {
        self.setup_deadline = Some(TokioInstant::now() + timeout);
    }

    fn begin_setup(&mut self, timeout: Duration) {
        self.session_healthy = false;
        self.arm_setup_deadline(timeout);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubscriptionCommand {
    Subscribe,
    Unsubscribe,
}

impl SubscriptionCommand {
    fn as_str(self) -> &'static str {
        match self {
            Self::Subscribe => "SUBSCRIBE",
            Self::Unsubscribe => "UNSUBSCRIBE",
        }
    }

    fn matches_response(self, subscribed: bool) -> bool {
        subscribed == (self == Self::Subscribe)
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "SUBSCRIBE" => Some(Self::Subscribe),
            "UNSUBSCRIBE" => Some(Self::Unsubscribe),
            _ => None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct PendingSubscription {
    command: SubscriptionCommand,
    event: VoiceSubscriptionEvent,
    channel_id: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn run_provider(
    mut backend: impl DiscordBackend,
    mut oauth: impl DiscordOauth,
    mut auth: DiscordAuth,
    client_id: String,
    rpc_paths: Vec<PathBuf>,
    publisher: LatestPublisher<DiscordSnapshot>,
    mut commands: mpsc::Receiver<DiscordCommand>,
    mut gate: watch::Receiver<DiscordGate>,
    mut shutdown: watch::Receiver<bool>,
    request_repaint: Arc<dyn Fn() + Send + Sync>,
    timing: WorkerTiming,
    mut diagnostics: ProviderDiagnostics,
) {
    let configured = valid_client_id(&client_id);
    let mut state = WorkerState::new(configured);
    publish(&mut state, &publisher, request_repaint.as_ref());
    let mut backoff = timing.initial_backoff;

    loop {
        if *shutdown.borrow() {
            break;
        }
        let current_gate = gate.borrow().clone();
        if !current_gate.is_open() {
            state.suspend_for_gate(&current_gate);
            publish(&mut state, &publisher, request_repaint.as_ref());
            if !wait_for_open_gate(
                &mut commands,
                &mut auth,
                &mut oauth,
                &mut gate,
                &mut shutdown,
                &mut state,
                &publisher,
                request_repaint.as_ref(),
                &mut diagnostics,
            )
            .await
            {
                break;
            }
        }
        if !state.auth_loaded {
            if auth.restore().is_err() {
                diagnostics.failed(FailureCategory::Identity);
            }
            state.auth_loaded = true;
            update_auth_snapshot(&mut state, &auth);
            publish(&mut state, &publisher, request_repaint.as_ref());
        }
        if !configured {
            state.snapshot.connection = DiscordConnectionState::ClientNotConfigured;
            publish(&mut state, &publisher, request_repaint.as_ref());
            if !wait_while_unconfigured(
                &mut commands,
                &mut auth,
                &mut oauth,
                &mut gate,
                &mut shutdown,
                &mut state,
                &publisher,
                request_repaint.as_ref(),
                &mut diagnostics,
            )
            .await
            {
                break;
            }
            continue;
        }

        state.snapshot.connection = DiscordConnectionState::Connecting;
        publish(&mut state, &publisher, request_repaint.as_ref());
        let socket = discover(&mut backend, &client_id, &rpc_paths, timing.connect_timeout).await;
        let Ok(mut socket) = socket else {
            state.snapshot.connection = DiscordConnectionState::DiscordUnavailable;
            publish(&mut state, &publisher, request_repaint.as_ref());
            diagnostics.failed(FailureCategory::Discovery);
            if !interruptible_delay(backoff, &mut gate, &mut shutdown).await {
                break;
            }
            backoff = (backoff * 2).min(timing.maximum_backoff);
            continue;
        };

        state.arm_setup_deadline(timing.operation_timeout);
        let outcome = run_socket(
            socket.as_mut(),
            &mut oauth,
            &mut auth,
            &client_id,
            &mut state,
            &publisher,
            &mut commands,
            &mut gate,
            &mut shutdown,
            request_repaint.as_ref(),
            timing.operation_timeout,
            timing.authorization_timeout,
            &mut diagnostics,
        )
        .await;
        let session_was_healthy = state.session_healthy;
        let _ = tokio::time::timeout(timing.operation_timeout, socket.close()).await;
        match outcome {
            SocketOutcome::Shutdown => break,
            SocketOutcome::Restart => {
                backoff = timing.initial_backoff;
                continue;
            }
            SocketOutcome::Reconnect(category) => {
                if session_was_healthy {
                    backoff = timing.initial_backoff;
                }
                if state.snapshot.connection == DiscordConnectionState::Failed {
                    state.clear_private_state();
                } else {
                    state.reset_rpc_session();
                }
                state.snapshot.connection = DiscordConnectionState::Connecting;
                publish(&mut state, &publisher, request_repaint.as_ref());
                diagnostics.failed(category);
                if !interruptible_delay(backoff, &mut gate, &mut shutdown).await {
                    break;
                }
                backoff = (backoff * 2).min(timing.maximum_backoff);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn wait_for_open_gate(
    commands: &mut mpsc::Receiver<DiscordCommand>,
    auth: &mut DiscordAuth,
    oauth: &mut dyn DiscordOauth,
    gate: &mut watch::Receiver<DiscordGate>,
    shutdown: &mut watch::Receiver<bool>,
    state: &mut WorkerState,
    publisher: &LatestPublisher<DiscordSnapshot>,
    request_repaint: &dyn Fn(),
    diagnostics: &mut ProviderDiagnostics,
) -> bool {
    loop {
        tokio::select! {
            changed = gate.changed() => {
                if changed.is_err() || gate.borrow().is_open() {
                    return changed.is_ok();
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return false;
                }
            }
            command = commands.recv() => {
                match command {
                    Some(DiscordCommand::SignOut) => {
                        process_sign_out(
                            auth,
                            oauth,
                            state,
                            DiscordConnectionState::Inert,
                            publisher,
                            request_repaint,
                            diagnostics,
                        );
                    }
                    Some(DiscordCommand::Connect) => {}
                    None => return false,
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn wait_while_unconfigured(
    commands: &mut mpsc::Receiver<DiscordCommand>,
    auth: &mut DiscordAuth,
    oauth: &mut dyn DiscordOauth,
    gate: &mut watch::Receiver<DiscordGate>,
    shutdown: &mut watch::Receiver<bool>,
    state: &mut WorkerState,
    publisher: &LatestPublisher<DiscordSnapshot>,
    request_repaint: &dyn Fn(),
    diagnostics: &mut ProviderDiagnostics,
) -> bool {
    loop {
        tokio::select! {
            changed = gate.changed() => return changed.is_ok(),
            changed = shutdown.changed() => {
                return changed.is_ok() && !*shutdown.borrow();
            }
            command = commands.recv() => {
                match command {
                    Some(DiscordCommand::SignOut) => {
                        process_sign_out(
                            auth,
                            oauth,
                            state,
                            DiscordConnectionState::ClientNotConfigured,
                            publisher,
                            request_repaint,
                            diagnostics,
                        );
                    }
                    Some(DiscordCommand::Connect) => {}
                    None => return false,
                }
            }
        }
    }
}

async fn interruptible_delay(
    duration: Duration,
    gate: &mut watch::Receiver<DiscordGate>,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => true,
        changed = gate.changed() => changed.is_ok(),
        changed = shutdown.changed() => changed.is_ok() && !*shutdown.borrow(),
    }
}

async fn discover(
    backend: &mut dyn DiscordBackend,
    client_id: &str,
    rpc_paths: &[PathBuf],
    timeout: Duration,
) -> Result<Box<dyn RpcSocket>, BackendError> {
    for path in rpc_paths {
        let attempt = ConnectAttempt {
            path: path.clone(),
            client_id: client_id.to_owned(),
        };
        match tokio::time::timeout(timeout, backend.connect(attempt)).await {
            Ok(Ok(socket)) => return Ok(socket),
            Ok(Err(_)) | Err(_) => continue,
        }
    }
    Err(BackendError::Connection)
}

enum SocketOutcome {
    Shutdown,
    Restart,
    Reconnect(FailureCategory),
}

#[allow(clippy::too_many_arguments)]
async fn run_socket(
    socket: &mut dyn RpcSocket,
    oauth: &mut dyn DiscordOauth,
    auth: &mut DiscordAuth,
    client_id: &str,
    state: &mut WorkerState,
    publisher: &LatestPublisher<DiscordSnapshot>,
    commands: &mut mpsc::Receiver<DiscordCommand>,
    gate: &mut watch::Receiver<DiscordGate>,
    shutdown: &mut watch::Receiver<bool>,
    request_repaint: &dyn Fn(),
    operation_timeout: Duration,
    authorization_timeout: Duration,
    diagnostics: &mut ProviderDiagnostics,
) -> SocketOutcome {
    loop {
        let setup_deadline = state.setup_deadline;
        tokio::select! {
            _ = wait_for_setup_deadline(setup_deadline) => {
                return SocketOutcome::Reconnect(FailureCategory::Timeout);
            }
            message = socket.next() => {
                let raw = match message {
                    Ok(Some(raw)) => raw,
                    Ok(None) => return SocketOutcome::Reconnect(FailureCategory::Connection),
                    Err(error) => {
                        return SocketOutcome::Reconnect(backend_failure_category(error));
                    }
                };
                let event = match parse_rpc_message(raw.as_bytes()) {
                    Ok(event) => event,
                    Err(error) => {
                        diagnostics.failed(match error {
                            RpcParseError::Oversized => FailureCategory::Content,
                            RpcParseError::Malformed => FailureCategory::Parse,
                            RpcParseError::InvalidData => FailureCategory::Validation,
                        });
                        continue;
                    }
                };
                if let Err(category) = handle_event(
                    event,
                    socket,
                    oauth,
                    auth,
                    state,
                    operation_timeout,
                    diagnostics,
                ).await {
                    return SocketOutcome::Reconnect(category);
                }
                let current_gate = gate.borrow().clone();
                if !current_gate.keeps_socket_open(state.snapshot.connection) {
                    state.suspend_for_gate(&current_gate);
                    publish(state, publisher, request_repaint);
                    return SocketOutcome::Restart;
                }
                publish(state, publisher, request_repaint);
            }
            command = commands.recv() => {
                match command {
                    Some(DiscordCommand::Connect)
                        if state.snapshot.connection == DiscordConnectionState::AuthorizationRequired =>
                    {
                        state.snapshot.connection = DiscordConnectionState::Authorizing;
                        let payload = json!({
                            "cmd": "AUTHORIZE",
                            "args": {"client_id": client_id, "scopes": ["identify", "rpc"]},
                        });
                        let nonce = match send_payload(socket, state, payload, operation_timeout).await {
                            Ok(nonce) => nonce,
                            Err(error) => {
                                return SocketOutcome::Reconnect(backend_failure_category(error));
                            }
                        };
                        state.pending_authorization_nonce = Some(nonce);
                        state.begin_setup(authorization_timeout);
                        publish(state, publisher, request_repaint);
                    }
                    Some(DiscordCommand::SignOut) => {
                        if process_sign_out(
                            auth,
                            oauth,
                            state,
                            DiscordConnectionState::AuthorizationRequired,
                            publisher,
                            request_repaint,
                            diagnostics,
                        ) {
                            return SocketOutcome::Restart;
                        }
                    }
                    Some(DiscordCommand::Connect) => {}
                    None => return SocketOutcome::Shutdown,
                }
            }
            changed = gate.changed() => {
                let current_gate = gate.borrow().clone();
                if changed.is_err() || !current_gate.keeps_socket_open(state.snapshot.connection) {
                    state.suspend_for_gate(&current_gate);
                    publish(state, publisher, request_repaint);
                    return SocketOutcome::Restart;
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return SocketOutcome::Shutdown;
                }
            }
        }
    }
}

async fn wait_for_setup_deadline(deadline: Option<TokioInstant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_event(
    event: DiscordRpcEvent,
    socket: &mut dyn RpcSocket,
    oauth: &mut dyn DiscordOauth,
    auth: &mut DiscordAuth,
    state: &mut WorkerState,
    timeout: Duration,
    diagnostics: &mut ProviderDiagnostics,
) -> Result<(), FailureCategory> {
    match event {
        DiscordRpcEvent::Ready => {
            if auth.access_token().is_some() {
                match auth.refresh_if_needed(oauth, unix_time_secs()) {
                    Ok(_) => send_authenticate(socket, state, auth, timeout)
                        .await
                        .map_err(backend_failure_category)?,
                    Err(AuthError::AuthorizationExpired) => {
                        state.snapshot.connection = DiscordConnectionState::AuthorizationRequired;
                        mark_authorization_ready(state, diagnostics);
                    }
                    Err(AuthError::Oauth(OauthError::Unauthorized)) => {
                        if auth.invalidate().is_ok() {
                            state.snapshot.connection =
                                DiscordConnectionState::AuthorizationRequired;
                            mark_authorization_ready(state, diagnostics);
                        } else {
                            state.snapshot.connection = DiscordConnectionState::Failed;
                            diagnostics.failed(FailureCategory::Identity);
                        }
                    }
                    Err(_) => {
                        state.snapshot.connection = DiscordConnectionState::Failed;
                        return Err(FailureCategory::Identity);
                    }
                }
            } else {
                state.snapshot.connection = DiscordConnectionState::AuthorizationRequired;
                mark_authorization_ready(state, diagnostics);
            }
            update_auth_snapshot(state, auth);
        }
        DiscordRpcEvent::AuthorizationGranted { nonce, code } => {
            if state.snapshot.connection != DiscordConnectionState::Authorizing
                || !take_matching_nonce(&mut state.pending_authorization_nonce, &nonce)
            {
                return Ok(());
            }
            match auth.authorize(code, oauth, unix_time_secs()) {
                Ok(_) => send_authenticate(socket, state, auth, timeout)
                    .await
                    .map_err(backend_failure_category)?,
                Err(_) => {
                    state.snapshot.connection = DiscordConnectionState::AuthorizationRequired;
                    diagnostics.failed(FailureCategory::Identity);
                    state.setup_deadline = None;
                }
            }
            update_auth_snapshot(state, auth);
        }
        DiscordRpcEvent::Authenticated { nonce, user_id } => {
            if state.snapshot.connection != DiscordConnectionState::Authenticating
                || !take_matching_nonce(&mut state.pending_authentication_nonce, &nonce)
            {
                return Ok(());
            }
            state.local_user_id = Some(user_id);
            state.authentication_refresh_attempted = false;
            state.snapshot.connection = DiscordConnectionState::Ready;
            let nonce = send_payload(
                socket,
                state,
                json!({"cmd": "SUBSCRIBE", "evt": "VOICE_CHANNEL_SELECT"}),
                timeout,
            )
            .await
            .map_err(backend_failure_category)?;
            state.pending_subscriptions.insert(
                nonce,
                PendingSubscription {
                    command: SubscriptionCommand::Subscribe,
                    event: VoiceSubscriptionEvent::VoiceChannelSelect,
                    channel_id: None,
                },
            );
            request_selected_channel(socket, state, timeout)
                .await
                .map_err(backend_failure_category)?;
        }
        DiscordRpcEvent::ChannelSelected(channel_id) => {
            if state.selected_channel_id.as_ref() != Some(&channel_id) {
                state.pending_subscriptions.retain(|_, pending| {
                    pending.command == SubscriptionCommand::Unsubscribe
                        || pending.channel_id.is_none()
                });
            }
            state.selected_channel_id = Some(channel_id.clone());
            if channel_id.as_deref()
                != state
                    .snapshot
                    .channel
                    .as_ref()
                    .map(|channel| channel.id.as_str())
            {
                state.snapshot.channel = None;
            }
            reconcile_channel_subscriptions(socket, state, timeout)
                .await
                .map_err(backend_failure_category)?;
            request_selected_channel(socket, state, timeout)
                .await
                .map_err(backend_failure_category)?;
        }
        DiscordRpcEvent::ChannelSnapshot { nonce, channel } => {
            if state.pending_channel_snapshot_nonce.as_deref() != Some(nonce.as_str()) {
                return Ok(());
            }
            state.pending_channel_snapshot_nonce = None;
            if let Some(expected) = state.selected_channel_id.as_ref()
                && channel.as_ref().map(|channel| channel.id.as_str()) != expected.as_deref()
            {
                return Err(FailureCategory::Validation);
            }
            state.channel_snapshot_received = true;
            state.snapshot.channel = channel;
            if let Some(channel) = state.snapshot.channel.as_mut() {
                sort_participants(&mut channel.participants, state.local_user_id.as_deref());
            }
            reconcile_channel_subscriptions(socket, state, timeout)
                .await
                .map_err(backend_failure_category)?;
            mark_session_healthy_if_ready(state, diagnostics);
        }
        DiscordRpcEvent::ParticipantCreated(participant) => {
            let local_user_id = state.local_user_id.clone();
            if let Some(channel) = active_channel_mut(state)
                && channel.participants.len() < 64
                && !channel
                    .participants
                    .iter()
                    .any(|current| current.id == participant.id)
            {
                channel.participants.push(participant);
                sort_participants(&mut channel.participants, local_user_id.as_deref());
            }
        }
        DiscordRpcEvent::ParticipantUpdated(participant) => {
            let local_user_id = state.local_user_id.clone();
            if let Some(channel) = active_channel_mut(state)
                && let Some(current) = channel
                    .participants
                    .iter_mut()
                    .find(|current| current.id == participant.id)
            {
                *current = participant;
                sort_participants(&mut channel.participants, local_user_id.as_deref());
            }
        }
        DiscordRpcEvent::ParticipantDeleted { user_id } => {
            if let Some(channel) = active_channel_mut(state) {
                channel
                    .participants
                    .retain(|participant| participant.id != user_id);
            }
            if state.local_user_id.as_deref() == Some(user_id.as_str()) {
                state.snapshot.channel = None;
            }
        }
        DiscordRpcEvent::SpeakingChanged { user_id, speaking } => {
            let local_user_id = state.local_user_id.clone();
            if let Some(participant) = active_participant_mut(state, &user_id) {
                participant.speaking = speaking;
            }
            if let Some(channel) = active_channel_mut(state) {
                sort_participants(&mut channel.participants, local_user_id.as_deref());
            }
        }
        DiscordRpcEvent::SubscriptionChanged {
            subscribed,
            event,
            nonce,
        } => {
            let Some(pending) = state.pending_subscriptions.remove(&nonce) else {
                return Ok(());
            };
            if pending.event != event || !pending.command.matches_response(subscribed) {
                return Err(FailureCategory::Validation);
            }
            if pending.command == SubscriptionCommand::Unsubscribe {
                reconcile_channel_subscriptions(socket, state, timeout)
                    .await
                    .map_err(backend_failure_category)?;
            }
            mark_session_healthy_if_ready(state, diagnostics);
        }
        DiscordRpcEvent::ProviderError {
            command,
            code,
            nonce,
        } if command.as_deref() == Some("AUTHENTICATE") => {
            if state.snapshot.connection != DiscordConnectionState::Authenticating {
                return Ok(());
            }
            let Some(nonce) = nonce else {
                return Err(FailureCategory::Validation);
            };
            if !take_matching_nonce(&mut state.pending_authentication_nonce, &nonce) {
                return Ok(());
            }
            if code != RPC_INVALID_TOKEN {
                state.snapshot.connection = DiscordConnectionState::Failed;
                return Err(FailureCategory::Response);
            }
            if state.authentication_refresh_attempted {
                match auth.invalidate() {
                    Ok(()) => {
                        state.clear_private_state();
                        state.snapshot.connection = DiscordConnectionState::AuthorizationRequired;
                        mark_authorization_ready(state, diagnostics);
                    }
                    Err(_) => {
                        state.snapshot.connection = DiscordConnectionState::Failed;
                        diagnostics.failed(FailureCategory::Identity);
                    }
                }
            } else {
                state.authentication_refresh_attempted = true;
                match auth.refresh_after_rejection(oauth, unix_time_secs()) {
                    Ok(_) => send_authenticate(socket, state, auth, timeout)
                        .await
                        .map_err(backend_failure_category)?,
                    Err(AuthError::AuthorizationExpired) => {
                        state.clear_private_state();
                        state.snapshot.connection = DiscordConnectionState::AuthorizationRequired;
                        mark_authorization_ready(state, diagnostics);
                    }
                    Err(_) => {
                        state.snapshot.connection = DiscordConnectionState::Failed;
                        return Err(FailureCategory::Identity);
                    }
                }
            }
            update_auth_snapshot(state, auth);
        }
        DiscordRpcEvent::ProviderError { command, nonce, .. }
            if command.as_deref() == Some("AUTHORIZE") =>
        {
            if state.snapshot.connection != DiscordConnectionState::Authorizing {
                return Ok(());
            }
            let Some(nonce) = nonce else {
                return Err(FailureCategory::Validation);
            };
            if !take_matching_nonce(&mut state.pending_authorization_nonce, &nonce) {
                return Ok(());
            }
            state.snapshot.connection = DiscordConnectionState::AuthorizationRequired;
            diagnostics.failed(FailureCategory::Identity);
            state.setup_deadline = None;
        }
        DiscordRpcEvent::ProviderError { command, nonce, .. }
            if matches!(command.as_deref(), Some("SUBSCRIBE" | "UNSUBSCRIBE")) =>
        {
            let Some(nonce) = nonce else {
                return Err(FailureCategory::Validation);
            };
            let Some(pending) = state.pending_subscriptions.get(&nonce) else {
                return Ok(());
            };
            if command.as_deref().and_then(SubscriptionCommand::parse) != Some(pending.command) {
                return Err(FailureCategory::Validation);
            }
            state.snapshot.connection = DiscordConnectionState::Failed;
            return Err(FailureCategory::Response);
        }
        DiscordRpcEvent::ProviderError { command, nonce, .. }
            if command.as_deref() == Some("GET_SELECTED_VOICE_CHANNEL") =>
        {
            let Some(nonce) = nonce else {
                return Err(FailureCategory::Validation);
            };
            if state.pending_channel_snapshot_nonce.as_deref() != Some(nonce.as_str()) {
                return Ok(());
            }
            state.snapshot.connection = DiscordConnectionState::Failed;
            return Err(FailureCategory::Response);
        }
        DiscordRpcEvent::ProviderError { .. } => {
            diagnostics.failed(FailureCategory::Response);
        }
        DiscordRpcEvent::Ignored => {}
    }
    Ok(())
}

fn backend_failure_category(error: BackendError) -> FailureCategory {
    match error {
        BackendError::Connection => FailureCategory::Connection,
        BackendError::Timeout => FailureCategory::Timeout,
        BackendError::Protocol => FailureCategory::Response,
    }
}

async fn send_authenticate(
    socket: &mut dyn RpcSocket,
    state: &mut WorkerState,
    auth: &DiscordAuth,
    timeout: Duration,
) -> Result<(), BackendError> {
    let access_token = auth.access_token().ok_or(BackendError::Protocol)?;
    state.snapshot.connection = DiscordConnectionState::Authenticating;
    let nonce = send_payload(
        socket,
        state,
        json!({"cmd": "AUTHENTICATE", "args": {"access_token": access_token}}),
        timeout,
    )
    .await?;
    state.pending_authentication_nonce = Some(nonce);
    state.arm_setup_deadline(timeout);
    Ok(())
}

fn take_matching_nonce(pending: &mut Option<String>, nonce: &str) -> bool {
    if pending.as_deref() != Some(nonce) {
        return false;
    }
    *pending = None;
    true
}

async fn send_payload(
    socket: &mut dyn RpcSocket,
    state: &mut WorkerState,
    mut payload: Value,
    timeout: Duration,
) -> Result<String, BackendError> {
    let object = payload.as_object_mut().ok_or(BackendError::Protocol)?;
    let nonce = state.next_nonce();
    object.insert("nonce".to_owned(), Value::String(nonce.clone()));
    let encoded = serde_json::to_string(&payload).map_err(|_| BackendError::Protocol)?;
    match tokio::time::timeout(timeout, socket.send(encoded)).await {
        Ok(Ok(())) => Ok(nonce),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(BackendError::Timeout),
    }
}

async fn request_selected_channel(
    socket: &mut dyn RpcSocket,
    state: &mut WorkerState,
    timeout: Duration,
) -> Result<(), BackendError> {
    let nonce = send_payload(
        socket,
        state,
        json!({"cmd": "GET_SELECTED_VOICE_CHANNEL"}),
        timeout,
    )
    .await?;
    state.pending_channel_snapshot_nonce = Some(nonce);
    state.channel_snapshot_received = false;
    state.arm_setup_deadline(timeout);
    Ok(())
}

async fn reconcile_channel_subscriptions(
    socket: &mut dyn RpcSocket,
    state: &mut WorkerState,
    timeout: Duration,
) -> Result<(), BackendError> {
    if state
        .pending_subscriptions
        .values()
        .any(|pending| pending.command == SubscriptionCommand::Unsubscribe)
    {
        return Ok(());
    }
    let next_channel_id = state
        .snapshot
        .channel
        .as_ref()
        .map(|channel| channel.id.clone());
    if state.subscribed_channel_id == next_channel_id {
        return Ok(());
    }
    if let Some(previous_channel_id) = state.subscribed_channel_id.take() {
        for event in VoiceSubscriptionEvent::CHANNEL_SCOPED {
            send_subscription(
                socket,
                state,
                SubscriptionCommand::Unsubscribe,
                &previous_channel_id,
                event,
                timeout,
            )
            .await?;
        }
        return Ok(());
    }
    let Some(next_channel_id) = next_channel_id else {
        return Ok(());
    };
    for event in VoiceSubscriptionEvent::CHANNEL_SCOPED {
        send_subscription(
            socket,
            state,
            SubscriptionCommand::Subscribe,
            &next_channel_id,
            event,
            timeout,
        )
        .await?;
    }
    state.subscribed_channel_id = Some(next_channel_id);
    Ok(())
}

fn mark_session_healthy_if_ready(state: &mut WorkerState, diagnostics: &mut ProviderDiagnostics) {
    if state.channel_snapshot_received && state.pending_subscriptions.is_empty() {
        state.setup_deadline = None;
        if !state.session_healthy {
            state.session_healthy = true;
            diagnostics.recovered();
        }
    }
}

fn mark_authorization_ready(state: &mut WorkerState, diagnostics: &mut ProviderDiagnostics) {
    state.setup_deadline = None;
    if !state.session_healthy {
        state.session_healthy = true;
        diagnostics.recovered();
    }
}

async fn send_subscription(
    socket: &mut dyn RpcSocket,
    state: &mut WorkerState,
    command: SubscriptionCommand,
    channel_id: &str,
    event: VoiceSubscriptionEvent,
    timeout: Duration,
) -> Result<(), BackendError> {
    let nonce = send_payload(
        socket,
        state,
        json!({"cmd": command.as_str(), "args": {"channel_id": channel_id}, "evt": event.as_str()}),
        timeout,
    )
    .await?;
    state.pending_subscriptions.insert(
        nonce,
        PendingSubscription {
            command,
            event,
            channel_id: Some(channel_id.to_owned()),
        },
    );
    state.arm_setup_deadline(timeout);
    Ok(())
}

fn active_channel_mut(state: &mut WorkerState) -> Option<&mut VoiceChannel> {
    let subscribed_channel_id = state.subscribed_channel_id.as_deref()?;
    let channel = state.snapshot.channel.as_mut()?;
    (channel.id == subscribed_channel_id).then_some(channel)
}

fn active_participant_mut<'a>(
    state: &'a mut WorkerState,
    user_id: &str,
) -> Option<&'a mut VoiceParticipant> {
    active_channel_mut(state)?
        .participants
        .iter_mut()
        .find(|participant| participant.id == user_id)
}

#[allow(clippy::too_many_arguments)]
fn process_sign_out(
    auth: &mut DiscordAuth,
    oauth: &mut dyn DiscordOauth,
    state: &mut WorkerState,
    success_state: DiscordConnectionState,
    publisher: &LatestPublisher<DiscordSnapshot>,
    request_repaint: &dyn Fn(),
    diagnostics: &mut ProviderDiagnostics,
) -> bool {
    let deleted = auth.sign_out(oauth).is_ok();
    if deleted {
        state.clear_private_state();
        state.snapshot.connection = success_state;
        diagnostics.recovered();
    } else {
        diagnostics.failed(FailureCategory::Identity);
    }
    state.auth_loaded = true;
    update_auth_snapshot(state, auth);
    publish(state, publisher, request_repaint);
    deleted
}

fn update_auth_snapshot(state: &mut WorkerState, auth: &DiscordAuth) {
    state.snapshot.credentials_available = auth.access_token().is_some();
    state.snapshot.credentials_persisted = auth.credentials_persisted();
}

fn publish(
    state: &mut WorkerState,
    publisher: &LatestPublisher<DiscordSnapshot>,
    request_repaint: &dyn Fn(),
) {
    state.snapshot.generation = state.snapshot.generation.wrapping_add(1);
    if publisher.publish(state.snapshot.clone()) {
        request_repaint();
    }
}

fn valid_client_id(value: &str) -> bool {
    (17..=32).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
