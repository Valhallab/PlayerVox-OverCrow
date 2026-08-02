use std::{
    collections::VecDeque,
    fs,
    future::pending,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use overcrow_logging::{Component, EventLogger, LoggerRuntime};
use tempfile::tempdir;
use tokio::io::duplex;

use super::{
    client::{
        BackendError, BackendFuture, ConnectAttempt, DiscordBackend, DiscordClient, DiscordCommand,
        DiscordConnectionState, DiscordGate, DiscordIpcSocket, DiscordSnapshot, ProductionBackend,
        RpcSocket, SocketFuture, WorkerTiming, configured_discord_client_id, is_owned_unix_socket,
        peer_uid_is_current, read_ipc_packet, rpc_socket_candidates, write_ipc_packet,
    },
    credentials::{CredentialStore, CredentialStoreError, DiscordCredentials},
    oauth::{DiscordOauth, OauthError, TokenResponse},
};

const CLIENT_ID: &str = "123456789012345678";

#[test]
fn official_build_has_a_discord_client_identity_without_manual_configuration() {
    assert_eq!(configured_discord_client_id(), "1533203091757858936");
}

fn rpc_paths() -> Vec<PathBuf> {
    (0..10)
        .map(|slot| PathBuf::from(format!("/tmp/discord-ipc-{slot}")))
        .collect()
}

#[tokio::test]
async fn native_rpc_frames_are_little_endian_bounded_and_round_trip() {
    let (mut writer, mut reader) = duplex(512);
    let payload = br#"{"cmd":"DISPATCH","evt":"READY","data":{}}"#;

    write_ipc_packet(&mut writer, 1, payload).await.unwrap();
    let (opcode, decoded) = read_ipc_packet(&mut reader).await.unwrap().unwrap();

    assert_eq!(opcode, 1);
    assert_eq!(decoded, payload);

    let mut oversized_header = Vec::new();
    oversized_header.extend_from_slice(&1_u32.to_le_bytes());
    oversized_header.extend_from_slice(&(256_u32 * 1024 + 1).to_le_bytes());
    let (mut writer, mut reader) = duplex(16);
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        writer.write_all(&oversized_header).await.unwrap();
    });
    assert_eq!(
        read_ipc_packet(&mut reader).await,
        Err(BackendError::Protocol)
    );
}

#[tokio::test]
async fn native_rpc_socket_replies_to_ping_and_rejects_unknown_opcodes() {
    let (client, mut peer) = duplex(512);
    let mut socket = DiscordIpcSocket { stream: client };
    write_ipc_packet(&mut peer, 3, b"heartbeat").await.unwrap();
    write_ipc_packet(&mut peer, 1, br#"{"cmd":"DISPATCH"}"#)
        .await
        .unwrap();

    assert_eq!(
        socket.next().await.unwrap(),
        Some(r#"{"cmd":"DISPATCH"}"#.to_owned())
    );
    assert_eq!(
        read_ipc_packet(&mut peer).await.unwrap(),
        Some((4, b"heartbeat".to_vec()))
    );

    let (client, mut peer) = duplex(32);
    let mut socket = DiscordIpcSocket { stream: client };
    write_ipc_packet(&mut peer, 99, b"unknown").await.unwrap();
    assert_eq!(socket.next().await, Err(BackendError::Protocol));
}

#[tokio::test]
async fn production_backend_sends_the_native_discord_handshake() {
    let root = tempdir().unwrap();
    let path = root.path().join("discord-ipc-0");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_ipc_packet(&mut stream).await.unwrap().unwrap()
    });

    let mut backend = ProductionBackend;
    let _socket = backend
        .connect(ConnectAttempt {
            path,
            client_id: CLIENT_ID.to_owned(),
        })
        .await
        .unwrap();
    let (opcode, payload) = server.await.unwrap();

    assert_eq!(opcode, 0);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&payload).unwrap(),
        serde_json::json!({"v": 1, "client_id": CLIENT_ID})
    );
}

#[test]
fn native_rpc_candidates_are_exact_absolute_and_deduplicated() {
    let candidates = rpc_socket_candidates(&[
        PathBuf::from("relative"),
        PathBuf::from("/run/user/1000"),
        PathBuf::from("/run/user/1000"),
        PathBuf::from("/tmp"),
    ]);

    assert_eq!(candidates.len(), 20);
    assert_eq!(candidates[0], Path::new("/run/user/1000/discord-ipc-0"));
    assert_eq!(candidates[9], Path::new("/run/user/1000/discord-ipc-9"));
    assert_eq!(candidates[10], Path::new("/tmp/discord-ipc-0"));
    assert_eq!(candidates[19], Path::new("/tmp/discord-ipc-9"));
}

#[test]
fn native_rpc_metadata_accepts_only_an_owned_unix_socket() {
    let root = tempdir().unwrap();
    let socket = root.path().join("discord-ipc-0");
    let regular = root.path().join("regular");
    let symlink = root.path().join("linked");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::fs::write(&regular, b"not a socket").unwrap();
    std::os::unix::fs::symlink(&socket, &symlink).unwrap();

    assert!(is_owned_unix_socket(&socket));
    assert!(!is_owned_unix_socket(&regular));
    assert!(!is_owned_unix_socket(&symlink));
    assert!(!is_owned_unix_socket(&root.path().join("missing")));
}

#[test]
fn connected_rpc_peer_must_match_the_effective_user() {
    // SAFETY: `geteuid` has no preconditions and does not retain pointers.
    let current_uid = unsafe { libc::geteuid() };
    assert!(peer_uid_is_current(current_uid));
    assert!(!peer_uid_is_current(current_uid.wrapping_add(1)));
}

#[derive(Default)]
struct StoreState {
    credentials: Option<DiscordCredentials>,
    loads: usize,
    deletes: usize,
    fail_delete: bool,
}

#[derive(Default)]
struct FakeStore(Mutex<StoreState>);

impl CredentialStore for FakeStore {
    fn load(&self) -> Result<Option<DiscordCredentials>, CredentialStoreError> {
        let mut state = self.0.lock().expect("credential store");
        state.loads += 1;
        Ok(state
            .credentials
            .as_ref()
            .map(DiscordCredentials::duplicate))
    }

    fn save(&self, credentials: &DiscordCredentials) -> Result<(), CredentialStoreError> {
        self.0.lock().expect("credential store").credentials = Some(credentials.duplicate());
        Ok(())
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        let mut state = self.0.lock().expect("credential store");
        state.deletes += 1;
        if state.fail_delete {
            Err(CredentialStoreError::Unavailable)
        } else {
            state.credentials = None;
            Ok(())
        }
    }
}

#[derive(Default)]
struct FakeOauthState {
    exchanges: usize,
    refreshes: usize,
    exchange_result: Option<Result<TokenResponse, OauthError>>,
    refresh_results: VecDeque<Result<TokenResponse, OauthError>>,
}

#[derive(Clone, Default)]
struct FakeOauth(Arc<Mutex<FakeOauthState>>);

impl DiscordOauth for FakeOauth {
    fn exchange(&mut self, _authorization_code: &str) -> Result<TokenResponse, OauthError> {
        let mut state = self.0.lock().expect("oauth state");
        state.exchanges += 1;
        state.exchange_result.take().unwrap_or_else(|| {
            Ok(TokenResponse {
                access_token: "new-access".to_owned(),
                refresh_token: "new-refresh".to_owned(),
                expires_in_secs: 7_200,
            })
        })
    }

    fn refresh(&mut self, _refresh_token: &str) -> Result<TokenResponse, OauthError> {
        let mut state = self.0.lock().expect("oauth state");
        state.refreshes += 1;
        state
            .refresh_results
            .pop_front()
            .unwrap_or(Err(OauthError::Unauthorized))
    }

    fn revoke(&mut self, _access_token: &str) -> Result<(), OauthError> {
        Ok(())
    }
}

struct FakeSocket {
    incoming: tokio::sync::mpsc::UnboundedReceiver<Result<String, BackendError>>,
    sent: mpsc::SyncSender<String>,
    dropped: Option<mpsc::SyncSender<()>>,
}

impl RpcSocket for FakeSocket {
    fn next(&mut self) -> SocketFuture<'_, Option<String>> {
        Box::pin(async move { self.incoming.recv().await.transpose() })
    }

    fn send(&mut self, message: String) -> SocketFuture<'_, ()> {
        let sent = self.sent.clone();
        Box::pin(async move { sent.send(message).map_err(|_| BackendError::Connection) })
    }

    fn close(&mut self) -> SocketFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

impl Drop for FakeSocket {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

enum ConnectBehavior {
    Fail,
    Pending,
    Socket(FakeSocket),
}

struct FakeBackend {
    behaviors: VecDeque<ConnectBehavior>,
    attempts: mpsc::SyncSender<ConnectAttempt>,
}

impl DiscordBackend for FakeBackend {
    fn connect(&mut self, attempt: ConnectAttempt) -> BackendFuture<'_, Box<dyn RpcSocket>> {
        let _ = self.attempts.send(attempt);
        let behavior = self.behaviors.pop_front().unwrap_or(ConnectBehavior::Fail);
        Box::pin(async move {
            match behavior {
                ConnectBehavior::Fail => Err(BackendError::Connection),
                ConnectBehavior::Pending => {
                    pending::<Result<Box<dyn RpcSocket>, BackendError>>().await
                }
                ConnectBehavior::Socket(socket) => Ok(Box::new(socket) as Box<dyn RpcSocket>),
            }
        })
    }
}

struct Harness {
    client: DiscordClient,
    incoming: Vec<tokio::sync::mpsc::UnboundedSender<Result<String, BackendError>>>,
    sent: mpsc::Receiver<String>,
    attempts: mpsc::Receiver<ConnectAttempt>,
    dropped: mpsc::Receiver<()>,
}

fn socket(
    sent: &mpsc::SyncSender<String>,
    dropped: &mpsc::SyncSender<()>,
) -> (
    FakeSocket,
    tokio::sync::mpsc::UnboundedSender<Result<String, BackendError>>,
) {
    let (incoming, receiver) = tokio::sync::mpsc::unbounded_channel();
    (
        FakeSocket {
            incoming: receiver,
            sent: sent.clone(),
            dropped: Some(dropped.clone()),
        },
        incoming,
    )
}

fn harness(
    behaviors: Vec<ConnectBehavior>,
    incoming: Vec<tokio::sync::mpsc::UnboundedSender<Result<String, BackendError>>>,
    store: Arc<FakeStore>,
) -> Harness {
    harness_with_oauth(behaviors, incoming, store, FakeOauth::default(), CLIENT_ID)
}

fn harness_with_oauth(
    behaviors: Vec<ConnectBehavior>,
    incoming: Vec<tokio::sync::mpsc::UnboundedSender<Result<String, BackendError>>>,
    store: Arc<FakeStore>,
    oauth: FakeOauth,
    client_id: &str,
) -> Harness {
    harness_with_timing(
        behaviors,
        incoming,
        store,
        oauth,
        client_id,
        WorkerTiming::for_tests(Duration::from_millis(40), Duration::from_millis(10)),
    )
}

fn harness_with_timing(
    behaviors: Vec<ConnectBehavior>,
    incoming: Vec<tokio::sync::mpsc::UnboundedSender<Result<String, BackendError>>>,
    store: Arc<FakeStore>,
    oauth: FakeOauth,
    client_id: &str,
    timing: WorkerTiming,
) -> Harness {
    harness_with_timing_and_logger(
        behaviors,
        incoming,
        store,
        oauth,
        client_id,
        timing,
        EventLogger::disabled(),
    )
}

#[allow(clippy::too_many_arguments)]
fn harness_with_timing_and_logger(
    behaviors: Vec<ConnectBehavior>,
    incoming: Vec<tokio::sync::mpsc::UnboundedSender<Result<String, BackendError>>>,
    store: Arc<FakeStore>,
    oauth: FakeOauth,
    client_id: &str,
    timing: WorkerTiming,
    logger: EventLogger,
) -> Harness {
    let (attempt_tx, attempts) = mpsc::sync_channel(32);
    let (sent_tx, sent) = mpsc::sync_channel(64);
    let (drop_tx, dropped) = mpsc::sync_channel(8);
    let backend = FakeBackend {
        behaviors: behaviors
            .into_iter()
            .map(|behavior| match behavior {
                ConnectBehavior::Socket(mut socket) => {
                    socket.sent = sent_tx.clone();
                    socket.dropped = Some(drop_tx.clone());
                    ConnectBehavior::Socket(socket)
                }
                other => other,
            })
            .collect(),
        attempts: attempt_tx,
    };
    let client = DiscordClient::spawn_with_backend_and_logger(
        backend,
        oauth,
        client_id,
        rpc_paths(),
        store,
        logger,
        || {},
        timing,
    );
    Harness {
        client,
        incoming,
        sent,
        attempts,
        dropped,
    }
}

fn one_socket_harness(with_credentials: bool) -> Harness {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (socket, incoming) = socket(&placeholder_sent, &placeholder_drop);
    let store = Arc::new(FakeStore::default());
    if with_credentials {
        store.0.lock().expect("credential store").credentials = Some(
            DiscordCredentials::new("access", "refresh", u64::MAX - 1).expect("valid credentials"),
        );
    }
    harness(vec![ConnectBehavior::Socket(socket)], vec![incoming], store)
}

fn two_socket_harness_with_credentials() -> Harness {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (first, first_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (second, second_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let store = Arc::new(FakeStore::default());
    store.0.lock().expect("credential store").credentials = Some(
        DiscordCredentials::new("access", "refresh", u64::MAX - 1).expect("valid credentials"),
    );
    harness(
        vec![
            ConnectBehavior::Socket(first),
            ConnectBehavior::Socket(second),
        ],
        vec![first_incoming, second_incoming],
        store,
    )
}

fn two_socket_timeout_harness() -> Harness {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (first, first_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (second, second_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", u64::MAX - 1).unwrap());
    harness_with_timing(
        vec![
            ConnectBehavior::Socket(first),
            ConnectBehavior::Socket(second),
        ],
        vec![first_incoming, second_incoming],
        store,
        FakeOauth::default(),
        CLIENT_ID,
        WorkerTiming::for_tests(Duration::from_millis(20), Duration::from_millis(5)),
    )
}

fn expect_first_and_reconnect_attempt(harness: &Harness) {
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    harness
        .attempts
        .recv_timeout(Duration::from_millis(150))
        .expect("reconnect after the pending RPC phase timed out");
}

fn open_gate() -> DiscordGate {
    DiscordGate {
        lifecycle_enabled: true,
        active_game_authorized: true,
        widget_enabled: true,
    }
}

fn wait_for_snapshot(
    client: &DiscordClient,
    predicate: impl Fn(&DiscordSnapshot) -> bool,
) -> Arc<DiscordSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(snapshot) = client.take_latest()
            && predicate(&snapshot.value)
        {
            return snapshot.value;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for Discord snapshot"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn ready() -> String {
    r#"{"cmd":"DISPATCH","evt":"READY","data":{}}"#.to_owned()
}

fn authenticated(authentication: &serde_json::Value) -> String {
    serde_json::json!({
        "cmd": "AUTHENTICATE",
        "data": {"user": {"id": "42", "username": "Tester"}},
        "nonce": authentication["nonce"],
    })
    .to_string()
}

fn authorization_granted(authorization: &serde_json::Value) -> String {
    serde_json::json!({
        "cmd": "AUTHORIZE",
        "data": {"code": "one-time-code"},
        "nonce": authorization["nonce"],
    })
    .to_string()
}

fn channel_with_participants(nonce: &str) -> String {
    serde_json::json!({
        "cmd": "GET_SELECTED_VOICE_CHANNEL",
        "data": {
            "id": "9",
            "name": "Squad",
            "voice_states": [
                {"nick": "Alice", "speaking": false, "voice_state": {}, "user": {"id": "7", "username": "alice", "avatar": null}},
                {"nick": "Bob", "speaking": false, "voice_state": {"self_mute": true}, "user": {"id": "8", "username": "bob", "avatar": null}}
            ]
        },
        "nonce": nonce
    })
    .to_string()
}

fn channel_with_nonce(id: &str, name: &str, participant_name: &str, nonce: &str) -> String {
    serde_json::json!({
        "cmd": "GET_SELECTED_VOICE_CHANNEL",
        "data": {
            "id": id,
            "name": name,
            "voice_states": [{
                "nick": participant_name,
                "speaking": false,
                "voice_state": {},
                "user": {"id": "7", "username": "pilot", "avatar": null}
            }]
        },
        "nonce": nonce
    })
    .to_string()
}

fn subscription_ack_for(command: &serde_json::Value) -> String {
    serde_json::json!({
        "cmd": command["cmd"],
        "data": {"evt": command["evt"]},
        "nonce": command["nonce"]
    })
    .to_string()
}

fn provider_error(command: &str, code: i64) -> String {
    serde_json::json!({
        "cmd": command,
        "evt": "ERROR",
        "data": {"code": code, "message": "private provider detail"}
    })
    .to_string()
}

fn provider_error_with_nonce(command: &str, code: i64, nonce: &str) -> String {
    serde_json::json!({
        "cmd": command,
        "evt": "ERROR",
        "data": {"code": code, "message": "private provider detail"},
        "nonce": nonce
    })
    .to_string()
}

fn wait_until(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for condition");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn recv_command(sent: &mpsc::Receiver<String>, command: &str) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let raw = sent
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("Discord RPC command");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("valid RPC JSON");
        if value["cmd"] == command {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {command}");
    }
}

fn drive_to_channel_subscriptions(harness: &Harness, socket_index: usize) {
    harness.incoming[socket_index].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[socket_index]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    recv_command(&harness.sent, "SUBSCRIBE");
    let request = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[socket_index]
        .send(Ok(channel_with_nonce(
            "9",
            "Squad",
            "Pilot",
            request["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    for _ in 0..5 {
        recv_command(&harness.sent, "SUBSCRIBE");
    }
}

fn drive_to_healthy_session(harness: &Harness, socket_index: usize) {
    harness.incoming[socket_index].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[socket_index]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    let global_subscription = recv_command(&harness.sent, "SUBSCRIBE");
    let request = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[socket_index]
        .send(Ok(channel_with_nonce(
            "9",
            "Squad",
            "Pilot",
            request["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    let channel_subscriptions = (0..5)
        .map(|_| recv_command(&harness.sent, "SUBSCRIBE"))
        .collect::<Vec<_>>();
    harness.incoming[socket_index]
        .send(Ok(subscription_ack_for(&global_subscription)))
        .unwrap();
    for subscription in &channel_subscriptions {
        harness.incoming[socket_index]
            .send(Ok(subscription_ack_for(subscription)))
            .unwrap();
    }
}

#[test]
fn discovery_checks_the_bounded_socket_range_in_order_with_fixed_metadata() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (socket, incoming) = socket(&placeholder_sent, &placeholder_drop);
    let mut behaviors = (0..9).map(|_| ConnectBehavior::Fail).collect::<Vec<_>>();
    behaviors.push(ConnectBehavior::Socket(socket));
    let harness = harness(behaviors, vec![incoming], Arc::new(FakeStore::default()));

    harness.client.set_gate(open_gate());
    let attempts = (0..=9)
        .map(|_| {
            harness
                .attempts
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        attempts
            .iter()
            .map(|attempt| attempt.path.clone())
            .collect::<Vec<_>>(),
        rpc_paths()
    );
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.client_id == CLIENT_ID)
    );
}

#[test]
fn a_timed_out_socket_does_not_block_discovery() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (socket, incoming) = socket(&placeholder_sent, &placeholder_drop);
    let harness = harness(
        vec![ConnectBehavior::Pending, ConnectBehavior::Socket(socket)],
        vec![incoming],
        Arc::new(FakeStore::default()),
    );

    harness.client.set_gate(open_gate());

    assert_eq!(
        harness
            .attempts
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .path,
        PathBuf::from("/tmp/discord-ipc-0")
    );
    assert_eq!(
        harness
            .attempts
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .path,
        PathBuf::from("/tmp/discord-ipc-1")
    );
}

#[test]
fn a_silent_socket_before_ready_reconnects_after_the_setup_deadline() {
    let harness = two_socket_timeout_harness();
    harness.client.set_gate(open_gate());

    expect_first_and_reconnect_attempt(&harness);
}

#[test]
fn a_missing_authentication_response_reconnects_after_the_setup_deadline() {
    let harness = two_socket_timeout_harness();
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    harness.incoming[0].send(Ok(ready())).unwrap();
    recv_command(&harness.sent, "AUTHENTICATE");

    harness
        .attempts
        .recv_timeout(Duration::from_millis(150))
        .expect("reconnect after AUTHENTICATE timed out");
}

#[test]
fn a_missing_channel_response_reconnects_after_the_setup_deadline() {
    let harness = two_socket_timeout_harness();
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    harness.incoming[0].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[0]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    recv_command(&harness.sent, "SUBSCRIBE");
    recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");

    harness
        .attempts
        .recv_timeout(Duration::from_millis(150))
        .expect("reconnect after channel discovery timed out");
}

#[test]
fn missing_subscription_acknowledgements_reconnect_after_the_setup_deadline() {
    let harness = two_socket_timeout_harness();
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    drive_to_channel_subscriptions(&harness, 0);

    harness
        .attempts
        .recv_timeout(Duration::from_millis(150))
        .expect("reconnect after subscription acknowledgements timed out");
}

#[test]
fn a_healthy_idle_session_has_no_setup_deadline() {
    let harness = two_socket_timeout_harness();
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    drive_to_healthy_session(&harness, 0);
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Ready && snapshot.channel.is_some()
    });

    assert!(
        harness
            .attempts
            .recv_timeout(Duration::from_millis(60))
            .is_err(),
        "an initialized event-driven session must not time out while idle"
    );
}

#[test]
fn authorization_required_is_a_healthy_idle_transport_state() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (first, first_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (second, second_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let harness = harness_with_timing(
        vec![
            ConnectBehavior::Socket(first),
            ConnectBehavior::Socket(second),
        ],
        vec![first_incoming, second_incoming],
        Arc::new(FakeStore::default()),
        FakeOauth::default(),
        CLIENT_ID,
        WorkerTiming::for_tests(Duration::from_millis(20), Duration::from_millis(5)),
    );
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    harness.incoming[0].send(Ok(ready())).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });

    assert!(
        harness
            .attempts
            .recv_timeout(Duration::from_millis(60))
            .is_err(),
        "waiting for explicit authorization must not reconnect"
    );
}

#[test]
fn authorization_prompt_uses_a_distinct_bounded_human_deadline() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (first, first_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (second, second_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let harness = harness_with_timing(
        vec![
            ConnectBehavior::Socket(first),
            ConnectBehavior::Socket(second),
        ],
        vec![first_incoming, second_incoming],
        Arc::new(FakeStore::default()),
        FakeOauth::default(),
        CLIENT_ID,
        WorkerTiming::for_tests_with_authorization_timeout(
            Duration::from_millis(20),
            Duration::from_millis(100),
            Duration::from_millis(5),
        ),
    );
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    harness.incoming[0].send(Ok(ready())).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });
    assert!(harness.client.try_send(DiscordCommand::Connect));
    recv_command(&harness.sent, "AUTHORIZE");

    assert!(
        harness
            .attempts
            .recv_timeout(Duration::from_millis(40))
            .is_err(),
        "authorization must survive the machine-response timeout"
    );
    harness
        .attempts
        .recv_timeout(Duration::from_millis(150))
        .expect("authorization must remain ultimately bounded");
}

#[test]
fn successful_authorization_retry_reports_provider_recovery() {
    let temp = tempdir().expect("create log directory");
    let log_runtime =
        LoggerRuntime::start_in(Component::Overlay, temp.path()).expect("start test logger");
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (socket, incoming) = socket(&placeholder_sent, &placeholder_drop);
    let harness = harness_with_timing_and_logger(
        vec![ConnectBehavior::Socket(socket)],
        vec![incoming],
        Arc::new(FakeStore::default()),
        FakeOauth::default(),
        CLIENT_ID,
        WorkerTiming::for_tests(Duration::from_millis(100), Duration::from_millis(10)),
        log_runtime.logger(),
    );
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });

    assert!(harness.client.try_send(DiscordCommand::Connect));
    let authorization = recv_command(&harness.sent, "AUTHORIZE");
    harness.incoming[0]
        .send(Ok(provider_error_with_nonce(
            "AUTHORIZE",
            4000,
            authorization["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });

    assert!(harness.client.try_send(DiscordCommand::Connect));
    let authorization = recv_command(&harness.sent, "AUTHORIZE");
    harness.incoming[0]
        .send(Ok(authorization_granted(&authorization)))
        .unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[0]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    let global_subscription = recv_command(&harness.sent, "SUBSCRIBE");
    let request = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "9",
            "Squad",
            "Pilot",
            request["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    let channel_subscriptions = (0..5)
        .map(|_| recv_command(&harness.sent, "SUBSCRIBE"))
        .collect::<Vec<_>>();
    let generation_before_acknowledgements = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Ready && snapshot.channel.is_some()
    })
    .generation;
    harness.incoming[0]
        .send(Ok(subscription_ack_for(&global_subscription)))
        .unwrap();
    for subscription in &channel_subscriptions {
        harness.incoming[0]
            .send(Ok(subscription_ack_for(subscription)))
            .unwrap();
    }
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot
            .generation
            .wrapping_sub(generation_before_acknowledgements)
            >= 6
    });

    let Harness {
        client,
        incoming: _,
        sent: _,
        attempts: _,
        dropped,
    } = harness;
    drop(client);
    dropped
        .recv_timeout(Duration::from_secs(1))
        .expect("Discord worker socket shutdown");
    drop(log_runtime);

    let contents =
        fs::read_to_string(temp.path().join("overlay.log")).expect("read Discord diagnostic log");
    assert_eq!(contents.matches("widget_provider_failed").count(), 1);
    assert!(contents.contains(
        "widget_provider_failed widget=discord_voice provider=discord category=identity"
    ));
    assert!(contents.contains("widget_provider_recovered widget=discord_voice provider=discord"));
    assert!(!contents.contains("one-time-code"));
}

#[test]
fn restored_credentials_authenticate_and_subscribe_before_loading_the_channel() {
    let harness = one_socket_harness(true);
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();

    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    assert_eq!(authentication["args"]["access_token"], "access");
    harness.incoming[0]
        .send(Ok(authenticated(&authentication)))
        .unwrap();

    let subscription = recv_command(&harness.sent, "SUBSCRIBE");
    assert_eq!(subscription["evt"], "VOICE_CHANNEL_SELECT");
    recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
}

#[test]
fn authorization_is_user_initiated_then_exchanged_without_exposing_the_code() {
    let harness = one_socket_harness(false);
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });

    assert!(harness.client.try_send(DiscordCommand::Connect));
    let authorize = recv_command(&harness.sent, "AUTHORIZE");
    assert_eq!(authorize["args"]["client_id"], CLIENT_ID);
    assert_eq!(
        authorize["args"]["scopes"],
        serde_json::json!(["identify", "rpc"])
    );
    harness.incoming[0]
        .send(Ok(authorization_granted(&authorize)))
        .unwrap();

    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    assert_eq!(authentication["args"]["access_token"], "new-access");
}

#[test]
fn stale_authorization_response_is_ignored() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (socket, incoming) = socket(&placeholder_sent, &placeholder_drop);
    let oauth = FakeOauth::default();
    let harness = harness_with_oauth(
        vec![ConnectBehavior::Socket(socket)],
        vec![incoming],
        Arc::new(FakeStore::default()),
        oauth.clone(),
        CLIENT_ID,
    );
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });
    assert!(harness.client.try_send(DiscordCommand::Connect));
    let authorization = recv_command(&harness.sent, "AUTHORIZE");

    harness.incoming[0]
        .send(Ok(
            r#"{"cmd":"AUTHORIZE","data":{"code":"stale-code"},"nonce":"stale"}"#.to_owned(),
        ))
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(oauth.0.lock().unwrap().exchanges, 0);
    assert!(harness.sent.try_recv().is_err());

    harness.incoming[0]
        .send(Ok(authorization_granted(&authorization)))
        .unwrap();
    recv_command(&harness.sent, "AUTHENTICATE");
    assert_eq!(oauth.0.lock().unwrap().exchanges, 1);
}

#[test]
fn stale_authentication_success_and_error_are_ignored() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (socket, incoming) = socket(&placeholder_sent, &placeholder_drop);
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", u64::MAX - 1).unwrap());
    let oauth = FakeOauth::default();
    let harness = harness_with_oauth(
        vec![ConnectBehavior::Socket(socket)],
        vec![incoming],
        store,
        oauth.clone(),
        CLIENT_ID,
    );
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");

    harness.incoming[0]
        .send(Ok(
            r#"{"cmd":"AUTHENTICATE","data":{"user":{"id":"42","username":"Tester"}},"nonce":"stale"}"#.to_owned(),
        ))
        .unwrap();
    harness.incoming[0]
        .send(Ok(provider_error_with_nonce("AUTHENTICATE", 4009, "stale")))
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(oauth.0.lock().unwrap().refreshes, 0);
    assert!(harness.sent.try_recv().is_err());

    harness.incoming[0]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    recv_command(&harness.sent, "SUBSCRIBE");
}

#[test]
fn authorization_survives_the_discord_consent_window_taking_focus() {
    let harness = one_socket_harness(false);
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });

    assert!(harness.client.try_send(DiscordCommand::Connect));
    let authorization = recv_command(&harness.sent, "AUTHORIZE");

    harness.client.set_gate(DiscordGate {
        lifecycle_enabled: true,
        active_game_authorized: false,
        widget_enabled: true,
    });
    assert!(
        harness
            .dropped
            .recv_timeout(Duration::from_millis(25))
            .is_err(),
        "the Discord consent window must not tear down its own RPC socket"
    );

    harness.incoming[0]
        .send(Ok(authorization_granted(&authorization)))
        .unwrap();
    recv_command(&harness.sent, "AUTHENTICATE");
}

#[test]
fn disabling_the_widget_still_cancels_an_authorization_in_progress() {
    let harness = one_socket_harness(false);
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });

    assert!(harness.client.try_send(DiscordCommand::Connect));
    recv_command(&harness.sent, "AUTHORIZE");
    harness.client.set_gate(DiscordGate {
        lifecycle_enabled: true,
        active_game_authorized: false,
        widget_enabled: false,
    });

    harness
        .dropped
        .recv_timeout(Duration::from_secs(1))
        .expect("disabling the widget must close the RPC socket");
}

#[test]
fn channel_events_update_one_coalesced_snapshot() {
    let harness = one_socket_harness(true);
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[0]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    recv_command(&harness.sent, "SUBSCRIBE");
    let request = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[0]
        .send(Ok(channel_with_participants(
            request["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    for _ in 0..5 {
        recv_command(&harness.sent, "SUBSCRIBE");
    }
    harness.incoming[0]
        .send(Ok(
            r#"{"cmd":"DISPATCH","evt":"SPEAKING_START","data":{"user_id":"8"}}"#.to_owned(),
        ))
        .unwrap();

    let snapshot = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.channel.as_ref().is_some_and(|channel| {
            channel
                .participants
                .iter()
                .any(|participant| participant.id == "8" && participant.speaking)
        })
    });
    assert_eq!(
        snapshot.channel.as_ref().unwrap().participants[0].display_name,
        "Bob"
    );
    assert!(snapshot.channel.as_ref().unwrap().participants[0].muted);

    harness.incoming[0]
        .send(Ok(serde_json::json!({
            "cmd": "DISPATCH",
            "evt": "VOICE_STATE_UPDATE",
            "data": {
                "nick": "Bob",
                "voice_state": {"self_deaf": true},
                "user": {"id": "8", "username": "bob", "avatar": null}
            }
        })
        .to_string()))
        .unwrap();
    let updated = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.channel.as_ref().is_some_and(|channel| {
            channel
                .participants
                .iter()
                .any(|participant| participant.id == "8" && participant.deafened)
        })
    });
    let bob = updated
        .channel
        .as_ref()
        .unwrap()
        .participants
        .iter()
        .find(|participant| participant.id == "8")
        .unwrap();
    assert!(bob.deafened);
    assert!(!bob.muted);
}

#[test]
fn channel_switch_unsubscribes_before_subscribing_without_duplicate_ownership() {
    let harness = one_socket_harness(true);
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[0]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    recv_command(&harness.sent, "SUBSCRIBE");
    let initial = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");

    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "9",
            "Alpha",
            "Alpha pilot",
            initial["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    for _ in 0..5 {
        let subscribe = recv_command(&harness.sent, "SUBSCRIBE");
        assert_eq!(subscribe["args"]["channel_id"], "9");
    }

    harness.incoming[0]
        .send(Ok(
            r#"{"cmd":"DISPATCH","evt":"VOICE_CHANNEL_SELECT","data":{"channel_id":"9"}}"#
                .to_owned(),
        ))
        .unwrap();
    let refresh = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "9",
            "Alpha",
            "Updated pilot",
            refresh["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    assert!(
        harness
            .sent
            .recv_timeout(Duration::from_millis(30))
            .is_err()
    );

    harness.incoming[0]
        .send(Ok(
            r#"{"cmd":"DISPATCH","evt":"VOICE_CHANNEL_SELECT","data":{"channel_id":"10"}}"#
                .to_owned(),
        ))
        .unwrap();
    let unsubscriptions = (0..5)
        .map(|_| recv_command(&harness.sent, "UNSUBSCRIBE"))
        .collect::<Vec<_>>();
    for unsubscribe in &unsubscriptions {
        assert_eq!(unsubscribe["args"]["channel_id"], "9");
    }
    let replacement = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "10",
            "Bravo",
            "Bravo pilot",
            replacement["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    harness.incoming[0]
        .send(Ok(serde_json::json!({
            "cmd": "DISPATCH",
            "evt": "VOICE_STATE_UPDATE",
            "data": {
                "nick": "Stale pilot",
                "voice_state": {},
                "user": {"id": "7", "username": "pilot"}
            }
        })
        .to_string()))
        .unwrap();

    for unsubscribe in &unsubscriptions {
        harness.incoming[0]
            .send(Ok(subscription_ack_for(unsubscribe)))
            .unwrap();
    }
    for _ in 0..5 {
        let subscribe = recv_command(&harness.sent, "SUBSCRIBE");
        assert_eq!(subscribe["args"]["channel_id"], "10");
    }

    let snapshot = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot
            .channel
            .as_ref()
            .is_some_and(|channel| channel.id == "10")
    });
    assert_eq!(
        snapshot.channel.as_ref().unwrap().participants[0].display_name,
        "Bravo pilot"
    );
}

#[test]
fn stale_selected_channel_response_cannot_replace_the_latest_selection() {
    let harness = one_socket_harness(true);
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[0]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    recv_command(&harness.sent, "SUBSCRIBE");
    let initial = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "9",
            "Alpha",
            "Alpha pilot",
            initial["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    for _ in 0..5 {
        recv_command(&harness.sent, "SUBSCRIBE");
    }

    harness.incoming[0]
        .send(Ok(
            r#"{"cmd":"DISPATCH","evt":"VOICE_CHANNEL_SELECT","data":{"channel_id":"10"}}"#
                .to_owned(),
        ))
        .unwrap();
    let unsubscriptions = (0..5)
        .map(|_| recv_command(&harness.sent, "UNSUBSCRIBE"))
        .collect::<Vec<_>>();
    let request_b = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[0]
        .send(Ok(
            r#"{"cmd":"DISPATCH","evt":"VOICE_CHANNEL_SELECT","data":{"channel_id":"11"}}"#
                .to_owned(),
        ))
        .unwrap();
    let request_c = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");

    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "11",
            "Charlie",
            "Charlie pilot",
            request_c["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "10",
            "Bravo",
            "Bravo pilot",
            request_b["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    for unsubscribe in &unsubscriptions {
        harness.incoming[0]
            .send(Ok(subscription_ack_for(unsubscribe)))
            .unwrap();
    }
    for _ in 0..5 {
        let subscription = recv_command(&harness.sent, "SUBSCRIBE");
        assert_eq!(subscription["args"]["channel_id"], "11");
    }

    let snapshot = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot
            .channel
            .as_ref()
            .is_some_and(|channel| channel.id == "11")
    });
    assert_eq!(snapshot.channel.as_ref().unwrap().name, "Charlie");
}

#[test]
fn stale_subscription_responses_cannot_affect_the_replacement_channel() {
    let harness = one_socket_harness(true);
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    harness.incoming[0].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[0]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    recv_command(&harness.sent, "SUBSCRIBE");
    let initial = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "9",
            "Alpha",
            "Alpha pilot",
            initial["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    let stale_subscriptions = (0..5)
        .map(|_| recv_command(&harness.sent, "SUBSCRIBE"))
        .collect::<Vec<_>>();

    harness.incoming[0]
        .send(Ok(
            r#"{"cmd":"DISPATCH","evt":"VOICE_CHANNEL_SELECT","data":{"channel_id":"10"}}"#
                .to_owned(),
        ))
        .unwrap();
    let unsubscriptions = (0..5)
        .map(|_| recv_command(&harness.sent, "UNSUBSCRIBE"))
        .collect::<Vec<_>>();
    let replacement = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "10",
            "Bravo",
            "Bravo pilot",
            replacement["nonce"].as_str().unwrap(),
        )))
        .unwrap();

    harness.incoming[0]
        .send(Ok(provider_error_with_nonce(
            "SUBSCRIBE",
            4006,
            stale_subscriptions[0]["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    assert!(
        harness
            .attempts
            .recv_timeout(Duration::from_millis(10))
            .is_err(),
        "a superseded subscription error must not reconnect the active session"
    );

    for unsubscribe in &unsubscriptions {
        harness.incoming[0]
            .send(Ok(subscription_ack_for(unsubscribe)))
            .unwrap();
    }
    for _ in 0..5 {
        let subscription = recv_command(&harness.sent, "SUBSCRIBE");
        assert_eq!(subscription["args"]["channel_id"], "10");
    }
    for stale_subscription in &stale_subscriptions {
        harness.incoming[0]
            .send(Ok(subscription_ack_for(stale_subscription)))
            .unwrap();
    }

    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("replacement subscriptions must still require their own acknowledgements");
}

#[test]
fn subscription_provider_error_reconnects_instead_of_stalling_voice_updates() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (first, first_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (second, second_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", u64::MAX - 1).unwrap());
    let harness = harness(
        vec![
            ConnectBehavior::Socket(first),
            ConnectBehavior::Socket(second),
        ],
        vec![first_incoming, second_incoming],
        store,
    );
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    harness.incoming[0].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[0]
        .send(Ok(authenticated(&authentication)))
        .unwrap();
    recv_command(&harness.sent, "SUBSCRIBE");
    let request = recv_command(&harness.sent, "GET_SELECTED_VOICE_CHANNEL");
    harness.incoming[0]
        .send(Ok(channel_with_nonce(
            "9",
            "Squad",
            "Alice",
            request["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    for _ in 0..5 {
        recv_command(&harness.sent, "SUBSCRIBE");
    }

    harness.incoming[0]
        .send(Ok(provider_error("SUBSCRIBE", 4006)))
        .unwrap();

    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("reconnect after rejected voice subscription");
}

#[test]
fn repeated_pre_health_protocol_failures_increase_reconnect_backoff() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (first, first_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (second, second_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (third, third_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", u64::MAX - 1).unwrap());
    let harness = harness_with_timing(
        vec![
            ConnectBehavior::Socket(first),
            ConnectBehavior::Socket(second),
            ConnectBehavior::Socket(third),
        ],
        vec![first_incoming, second_incoming, third_incoming],
        store,
        FakeOauth::default(),
        CLIENT_ID,
        WorkerTiming::for_tests(Duration::from_millis(100), Duration::from_millis(20)),
    );
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("first connection attempt");

    drive_to_channel_subscriptions(&harness, 0);
    let first_failure = Instant::now();
    harness.incoming[0]
        .send(Ok(provider_error("SUBSCRIBE", 4006)))
        .unwrap();
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("second connection attempt");
    let first_delay = first_failure.elapsed();

    drive_to_channel_subscriptions(&harness, 1);
    let second_failure = Instant::now();
    harness.incoming[1]
        .send(Ok(provider_error("SUBSCRIBE", 4006)))
        .unwrap();
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("third connection attempt");
    let second_delay = second_failure.elapsed();

    assert!(first_delay >= Duration::from_millis(15), "{first_delay:?}");
    assert!(
        second_delay >= Duration::from_millis(35),
        "{second_delay:?}"
    );
}

#[test]
fn gate_closure_clears_private_state_and_interrupts_the_socket() {
    let harness = one_socket_harness(true);
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    recv_command(&harness.sent, "AUTHENTICATE");

    harness.client.set_gate(DiscordGate::default());

    harness
        .dropped
        .recv_timeout(Duration::from_secs(1))
        .expect("socket dropped");
    let snapshot = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Inert
    });
    assert!(snapshot.channel.is_none());
}

#[test]
fn focus_suspension_retains_channel_and_reopens_immediately() {
    let harness = two_socket_harness_with_credentials();
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    drive_to_healthy_session(&harness, 0);
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Ready && snapshot.channel.is_some()
    });

    harness.client.set_gate(DiscordGate {
        lifecycle_enabled: true,
        active_game_authorized: false,
        widget_enabled: true,
    });
    harness
        .dropped
        .recv_timeout(Duration::from_secs(1))
        .expect("focus suspension must close the live RPC socket");
    let suspended = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Inert
    });
    assert_eq!(
        suspended
            .channel
            .as_ref()
            .map(|channel| channel.name.as_str()),
        Some("Squad")
    );

    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_millis(150))
        .expect("refocus must bypass provider failure backoff");
    let reconnecting = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Connecting
    });
    assert_eq!(
        reconnecting
            .channel
            .as_ref()
            .map(|channel| channel.name.as_str()),
        Some("Squad")
    );

    harness.incoming[1]
        .send(Err(BackendError::Connection))
        .unwrap();
    harness
        .dropped
        .recv_timeout(Duration::from_secs(1))
        .expect("failed resynchronization socket dropped");
    let retrying = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Connecting
    });
    assert_eq!(
        retrying
            .channel
            .as_ref()
            .map(|channel| channel.name.as_str()),
        Some("Squad")
    );
}

#[test]
fn focus_suspension_does_not_retain_channel_when_the_widget_is_disabled() {
    let harness = one_socket_harness(true);
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    drive_to_healthy_session(&harness, 0);
    wait_for_snapshot(&harness.client, |snapshot| snapshot.channel.is_some());

    harness.client.set_gate(DiscordGate {
        lifecycle_enabled: true,
        active_game_authorized: false,
        widget_enabled: false,
    });
    let snapshot = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Inert
    });
    assert!(snapshot.channel.is_none());
}

#[test]
fn a_closed_socket_reconnects_and_shutdown_remains_bounded() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (first, first_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (second, second_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let harness = harness(
        vec![
            ConnectBehavior::Socket(first),
            ConnectBehavior::Socket(second),
        ],
        vec![first_incoming, second_incoming],
        Arc::new(FakeStore::default()),
    );
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    harness.incoming[0]
        .send(Err(BackendError::Connection))
        .unwrap();
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("reconnect attempt");

    let started = Instant::now();
    drop(harness.client);
    assert!(started.elapsed() < Duration::from_millis(100));
}

#[test]
fn healthy_socket_failure_retains_channel_while_resynchronizing() {
    let harness = two_socket_harness_with_credentials();
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("initial connection attempt");
    drive_to_healthy_session(&harness, 0);
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Ready && snapshot.channel.is_some()
    });

    harness.incoming[0]
        .send(Err(BackendError::Connection))
        .unwrap();
    let reconnecting = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::Connecting
    });

    assert_eq!(
        reconnecting
            .channel
            .as_ref()
            .map(|channel| channel.name.as_str()),
        Some("Squad")
    );
}

#[test]
fn credential_restore_is_deferred_until_all_gates_open() {
    let store = Arc::new(FakeStore::default());
    let harness = harness(vec![ConnectBehavior::Fail], Vec::new(), store.clone());

    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(store.0.lock().unwrap().loads, 0);

    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("RPC discovery after gate opens");
    assert_eq!(store.0.lock().unwrap().loads, 1);
}

#[test]
fn sign_out_is_processed_even_when_the_client_build_is_unconfigured() {
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", u64::MAX - 1).unwrap());
    let harness = harness_with_oauth(
        Vec::new(),
        Vec::new(),
        store.clone(),
        FakeOauth::default(),
        "",
    );
    harness.client.set_gate(open_gate());
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::ClientNotConfigured
            && snapshot.credentials_available
    });

    assert!(harness.client.try_send(DiscordCommand::SignOut));
    wait_until(|| store.0.lock().unwrap().deletes == 1);
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::ClientNotConfigured
            && !snapshot.credentials_available
    });
}

#[test]
fn a_short_numeric_client_id_is_not_treated_as_configured() {
    let harness = harness_with_oauth(
        Vec::new(),
        Vec::new(),
        Arc::new(FakeStore::default()),
        FakeOauth::default(),
        "1",
    );
    harness.client.set_gate(open_gate());

    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::ClientNotConfigured
    });
    assert!(
        harness
            .attempts
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
}

#[test]
fn invalid_token_refreshes_only_once_per_authentication_cycle() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (socket, incoming) = socket(&placeholder_sent, &placeholder_drop);
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", u64::MAX - 1).unwrap());
    let oauth = FakeOauth::default();
    {
        let mut state = oauth.0.lock().unwrap();
        state.refresh_results.push_back(Ok(TokenResponse {
            access_token: "rotated-access".to_owned(),
            refresh_token: "rotated-refresh".to_owned(),
            expires_in_secs: 7_200,
        }));
        state.refresh_results.push_back(Ok(TokenResponse {
            access_token: "must-not-be-used".to_owned(),
            refresh_token: "must-not-be-used".to_owned(),
            expires_in_secs: 7_200,
        }));
    }
    let harness = harness_with_oauth(
        vec![ConnectBehavior::Socket(socket)],
        vec![incoming],
        store,
        oauth.clone(),
        CLIENT_ID,
    );
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");

    harness.incoming[0]
        .send(Ok(provider_error_with_nonce(
            "AUTHENTICATE",
            4009,
            authentication["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    assert_eq!(authentication["args"]["access_token"], "rotated-access");
    harness.incoming[0]
        .send(Ok(provider_error_with_nonce(
            "AUTHENTICATE",
            4009,
            authentication["nonce"].as_str().unwrap(),
        )))
        .unwrap();

    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });
    assert_eq!(oauth.0.lock().unwrap().refreshes, 1);
}

#[test]
fn socket_reconnect_does_not_reset_the_authentication_refresh_guard() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (first, first_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (second, second_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", u64::MAX - 1).unwrap());
    let oauth = FakeOauth::default();
    oauth
        .0
        .lock()
        .unwrap()
        .refresh_results
        .push_back(Ok(TokenResponse {
            access_token: "rotated-access".to_owned(),
            refresh_token: "rotated-refresh".to_owned(),
            expires_in_secs: 7_200,
        }));
    let harness = harness_with_oauth(
        vec![
            ConnectBehavior::Socket(first),
            ConnectBehavior::Socket(second),
        ],
        vec![first_incoming, second_incoming],
        store,
        oauth.clone(),
        CLIENT_ID,
    );
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("first connection attempt");
    harness.incoming[0].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[0]
        .send(Ok(provider_error_with_nonce(
            "AUTHENTICATE",
            4009,
            authentication["nonce"].as_str().unwrap(),
        )))
        .unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    assert_eq!(authentication["args"]["access_token"], "rotated-access");
    harness.incoming[0]
        .send(Err(BackendError::Connection))
        .unwrap();

    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("second connection attempt");
    harness.incoming[1].send(Ok(ready())).unwrap();
    let authentication = recv_command(&harness.sent, "AUTHENTICATE");
    harness.incoming[1]
        .send(Ok(provider_error_with_nonce(
            "AUTHENTICATE",
            4009,
            authentication["nonce"].as_str().unwrap(),
        )))
        .unwrap();

    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });
    assert_eq!(oauth.0.lock().unwrap().refreshes, 1);
}

#[test]
fn failed_authorization_exchange_returns_to_a_user_retryable_state() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (socket, incoming) = socket(&placeholder_sent, &placeholder_drop);
    let oauth = FakeOauth::default();
    oauth.0.lock().unwrap().exchange_result = Some(Err(OauthError::Transport));
    let harness = harness_with_oauth(
        vec![ConnectBehavior::Socket(socket)],
        vec![incoming],
        Arc::new(FakeStore::default()),
        oauth,
        CLIENT_ID,
    );
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });
    assert!(harness.client.try_send(DiscordCommand::Connect));
    let authorization = recv_command(&harness.sent, "AUTHORIZE");
    harness.incoming[0]
        .send(Ok(authorization_granted(&authorization)))
        .unwrap();

    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });
}

#[test]
fn authorization_provider_error_returns_to_a_user_retryable_state() {
    let harness = one_socket_harness(false);
    harness.client.set_gate(open_gate());
    harness.incoming[0].send(Ok(ready())).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });
    assert!(harness.client.try_send(DiscordCommand::Connect));
    let authorization = recv_command(&harness.sent, "AUTHORIZE");
    harness.incoming[0]
        .send(Ok(provider_error_with_nonce(
            "AUTHORIZE",
            4006,
            authorization["nonce"].as_str().unwrap(),
        )))
        .unwrap();

    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == DiscordConnectionState::AuthorizationRequired
    });
}

#[test]
fn transient_refresh_failure_reconnects_with_bounded_backoff() {
    let (placeholder_sent, _) = mpsc::sync_channel(1);
    let (placeholder_drop, _) = mpsc::sync_channel(1);
    let (first, first_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let (second, second_incoming) = socket(&placeholder_sent, &placeholder_drop);
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", 1).unwrap());
    let oauth = FakeOauth::default();
    oauth
        .0
        .lock()
        .unwrap()
        .refresh_results
        .push_back(Err(OauthError::Transport));
    let harness = harness_with_oauth(
        vec![
            ConnectBehavior::Socket(first),
            ConnectBehavior::Socket(second),
        ],
        vec![first_incoming, second_incoming],
        store,
        oauth,
        CLIENT_ID,
    );
    harness.client.set_gate(open_gate());
    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    harness.incoming[0].send(Ok(ready())).unwrap();

    harness
        .attempts
        .recv_timeout(Duration::from_secs(1))
        .expect("bounded reconnect after transient broker failure");
}
