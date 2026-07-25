use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use super::{
    client::{
        BackendError, BackendFuture, EventSocket, SocketFuture, TwitchBackend, TwitchClient,
        TwitchGate, WorkerTiming, command_channel,
    },
    credentials::{CredentialStore, CredentialStoreError, TwitchCredentials},
    http::{
        ChatSendResult, ChatSubscription, DeviceCodeGrant, HttpError, TokenPoll, TokenResponse,
        TokenValidation, TwitchHttp, TwitchUser,
    },
    model::{
        TwitchCommand, TwitchConnectionState, TwitchSendReceiptState, TwitchSendState,
        TwitchSnapshot,
    },
};

const CLIENT_ID: &str = "public-client-id";
const INITIAL_EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";

struct FakeStore(TwitchCredentials);

impl CredentialStore for FakeStore {
    fn load(&self) -> Result<Option<TwitchCredentials>, CredentialStoreError> {
        Ok(Some(self.0.duplicate()))
    }

    fn save(&self, _credentials: &TwitchCredentials) -> Result<(), CredentialStoreError> {
        Ok(())
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        Ok(())
    }
}

#[derive(Default)]
struct HttpState {
    resolved_logins: Vec<String>,
    subscriptions: Vec<ChatSubscription>,
    sends: Vec<(String, String, Option<String>)>,
    send_results: VecDeque<Result<ChatSendResult, HttpError>>,
}

#[derive(Clone, Default)]
struct FakeHttp {
    state: Arc<Mutex<HttpState>>,
}

impl TwitchHttp for FakeHttp {
    fn begin_device_authorization(
        &mut self,
        _client_id: &str,
    ) -> Result<DeviceCodeGrant, HttpError> {
        Err(HttpError::ProviderResponse)
    }

    fn poll_device_token(
        &mut self,
        _client_id: &str,
        _device_code: &str,
    ) -> Result<TokenPoll, HttpError> {
        Err(HttpError::ProviderResponse)
    }

    fn refresh_token(
        &mut self,
        _client_id: &str,
        _refresh_token: &str,
    ) -> Result<TokenResponse, HttpError> {
        Err(HttpError::ProviderResponse)
    }

    fn validate_token(&mut self, _access_token: &str) -> Result<TokenValidation, HttpError> {
        Ok(TokenValidation {
            client_id: CLIENT_ID.to_owned(),
            login: "player_vox".to_owned(),
            user_id: "42".to_owned(),
            scopes: vec!["user:read:chat".to_owned(), "user:write:chat".to_owned()],
            expires_in_secs: 7_200,
        })
    }

    fn revoke_token(&mut self, _client_id: &str, _access_token: &str) -> Result<(), HttpError> {
        Ok(())
    }

    fn resolve_channel(
        &mut self,
        _client_id: &str,
        _access_token: &str,
        login: &str,
    ) -> Result<TwitchUser, HttpError> {
        self.state
            .lock()
            .expect("HTTP state")
            .resolved_logins
            .push(login.to_owned());
        Ok(TwitchUser {
            id: "100".to_owned(),
        })
    }

    fn create_chat_subscription(
        &mut self,
        _client_id: &str,
        _access_token: &str,
        _session_id: &str,
        _broadcaster_user_id: &str,
        _user_id: &str,
        subscription: ChatSubscription,
    ) -> Result<(), HttpError> {
        self.state
            .lock()
            .expect("HTTP state")
            .subscriptions
            .push(subscription);
        Ok(())
    }

    fn send_chat_message(
        &mut self,
        _client_id: &str,
        _access_token: &str,
        broadcaster_user_id: &str,
        _sender_id: &str,
        message: &str,
        reply_parent_message_id: Option<&str>,
    ) -> Result<ChatSendResult, HttpError> {
        let mut state = self.state.lock().expect("HTTP state");
        state.sends.push((
            broadcaster_user_id.to_owned(),
            message.to_owned(),
            reply_parent_message_id.map(str::to_owned),
        ));
        state.send_results.pop_front().unwrap_or_else(|| {
            Ok(ChatSendResult {
                message_id: Some("sent-message-1".to_owned()),
                is_sent: true,
            })
        })
    }
}

struct FakeSocket {
    incoming: tokio::sync::mpsc::UnboundedReceiver<Result<String, BackendError>>,
    dropped: Option<mpsc::SyncSender<()>>,
}

impl EventSocket for FakeSocket {
    fn next(&mut self) -> SocketFuture<'_, Option<String>> {
        Box::pin(async move { self.incoming.recv().await.transpose() })
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

struct FakeBackend {
    http: FakeHttp,
    sockets: VecDeque<FakeSocket>,
    connects: mpsc::SyncSender<String>,
}

impl TwitchBackend for FakeBackend {
    fn http(&mut self) -> &mut dyn TwitchHttp {
        &mut self.http
    }

    fn connect_eventsub(&mut self, url: &str) -> BackendFuture<'_, Box<dyn EventSocket>> {
        let _ = self.connects.send(url.to_owned());
        let socket = self.sockets.pop_front();
        Box::pin(async move {
            socket
                .map(|socket| Box::new(socket) as Box<dyn EventSocket>)
                .ok_or(BackendError::Connection)
        })
    }
}

struct Harness {
    client: TwitchClient,
    incoming: Vec<tokio::sync::mpsc::UnboundedSender<Result<String, BackendError>>>,
    connects: mpsc::Receiver<String>,
    dropped: mpsc::Receiver<()>,
    http: Arc<Mutex<HttpState>>,
}

fn harness(socket_count: usize) -> Harness {
    let http = FakeHttp::default();
    let http_state = Arc::clone(&http.state);
    let (connect_tx, connects) = mpsc::sync_channel(8);
    let (drop_tx, dropped) = mpsc::sync_channel(8);
    let mut sockets = VecDeque::new();
    let mut incoming = Vec::new();
    for _ in 0..socket_count {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        incoming.push(sender);
        sockets.push_back(FakeSocket {
            incoming: receiver,
            dropped: Some(drop_tx.clone()),
        });
    }
    let backend = FakeBackend {
        http,
        sockets,
        connects: connect_tx,
    };
    let store = Arc::new(FakeStore(
        TwitchCredentials::new("access-secret", "refresh-secret", u64::MAX - 1)
            .expect("credentials"),
    ));
    let client = TwitchClient::spawn_with_backend(
        backend,
        CLIENT_ID,
        store,
        || {},
        WorkerTiming::for_tests(Duration::from_millis(200), Duration::from_millis(10)),
    );
    Harness {
        client,
        incoming,
        connects,
        dropped,
        http: http_state,
    }
}

fn open_gate() -> TwitchGate {
    TwitchGate {
        lifecycle_enabled: true,
        active_game_authorized: true,
        widget_enabled: true,
        channel: Some("warframe".to_owned()),
    }
}

fn wait_for_snapshot(
    client: &TwitchClient,
    predicate: impl Fn(&TwitchSnapshot) -> bool,
) -> Arc<TwitchSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(snapshot) = client.take_latest()
            && predicate(&snapshot.value)
        {
            return snapshot.value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for snapshot");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn welcome(delivery: &str, session: &str) -> String {
    format!(
        r#"{{"metadata":{{"message_id":"{delivery}","message_type":"session_welcome"}},"payload":{{"session":{{"id":"{session}","keepalive_timeout_seconds":10}}}}}}"#
    )
}

fn chat(delivery: &str, message_id: &str, text: &str) -> String {
    format!(
        r##"{{"metadata":{{"message_id":"{delivery}","message_type":"notification","subscription_type":"channel.chat.message","subscription_version":"1"}},"payload":{{"event":{{"broadcaster_user_id":"100","chatter_user_id":"7","chatter_user_login":"alice","chatter_user_name":"Alice","message_id":"{message_id}","color":"#00FF00","message":{{"text":"{text}"}}}}}}}}"##
    )
}

fn join_chat(harness: &Harness, socket_index: usize) -> Arc<TwitchSnapshot> {
    harness.client.set_gate(open_gate());
    assert_eq!(
        harness
            .connects
            .recv_timeout(Duration::from_secs(1))
            .expect("EventSub connection"),
        INITIAL_EVENTSUB_URL
    );
    harness.incoming[socket_index]
        .send(Ok(welcome("welcome-1", "session-1")))
        .expect("welcome");
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == TwitchConnectionState::Joined
    })
}

#[test]
fn twitch_worker_is_inert_then_subscribes_to_an_arbitrary_public_channel() {
    let harness = harness(1);
    assert!(
        harness
            .connects
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );

    join_chat(&harness, 0);

    let state = harness.http.lock().expect("HTTP state");
    assert_eq!(state.resolved_logins, ["warframe"]);
    assert_eq!(
        state.subscriptions,
        [
            ChatSubscription::Message,
            ChatSubscription::Clear,
            ChatSubscription::MessageDelete,
        ]
    );
}

#[test]
fn twitch_worker_deduplicates_notifications_and_applies_delete_and_clear() {
    let harness = harness(1);
    join_chat(&harness, 0);

    let duplicate = chat("delivery-1", "message-1", "hello");
    harness.incoming[0].send(Ok(duplicate.clone())).unwrap();
    harness.incoming[0].send(Ok(duplicate)).unwrap();
    wait_for_snapshot(&harness.client, |snapshot| snapshot.messages.len() == 1);

    harness.incoming[0]
        .send(Ok(chat("delivery-2", "message-2", "second")))
        .unwrap();
    wait_for_snapshot(&harness.client, |snapshot| snapshot.messages.len() == 2);

    harness.incoming[0]
        .send(Ok(
            r#"{"metadata":{"message_id":"delivery-3","message_type":"notification","subscription_type":"channel.chat.message_delete","subscription_version":"1"},"payload":{"event":{"broadcaster_user_id":"100","message_id":"message-1"}}}"#.to_owned(),
        ))
        .unwrap();
    let snapshot = wait_for_snapshot(&harness.client, |snapshot| snapshot.messages.len() == 1);
    assert_eq!(snapshot.messages[0].id, "message-2");

    harness.incoming[0]
        .send(Ok(
            r#"{"metadata":{"message_id":"delivery-4","message_type":"notification","subscription_type":"channel.chat.clear","subscription_version":"1"},"payload":{"event":{"broadcaster_user_id":"100"}}}"#.to_owned(),
        ))
        .unwrap();
    wait_for_snapshot(&harness.client, |snapshot| snapshot.messages.is_empty());
}

#[test]
fn twitch_worker_drops_one_malformed_notification_without_reconnecting() {
    let harness = harness(1);
    join_chat(&harness, 0);

    harness.incoming[0]
        .send(Ok(
            r#"{"metadata":{"message_id":"bad-delivery","message_type":"notification","subscription_type":"channel.chat.message","subscription_version":"1"},"payload":{}}"#.to_owned(),
        ))
        .expect("malformed notification");
    assert!(
        harness
            .dropped
            .recv_timeout(Duration::from_millis(50))
            .is_err(),
        "a malformed notification must not close the healthy EventSub socket"
    );

    harness.incoming[0]
        .send(Ok(chat(
            "delivery-after-bad",
            "message-after-bad",
            "still live",
        )))
        .expect("valid notification");
    let snapshot = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot
            .messages
            .iter()
            .any(|message| message.id == "message-after-bad")
    });
    assert_eq!(snapshot.connection, TwitchConnectionState::Joined);
    assert!(
        harness
            .connects
            .recv_timeout(Duration::from_millis(50))
            .is_err(),
        "the original EventSub socket must remain in use"
    );
}

#[test]
fn twitch_send_uses_helix_result_and_reconciles_the_eventsub_echo() {
    let harness = harness(1);
    let joined = join_chat(&harness, 0);
    assert!(harness.client.try_send(TwitchCommand::SendMessage {
        request_id: 1,
        generation: joined.generation,
        channel: "warframe".to_owned(),
        text: "hello chat".to_owned(),
        reply_to: Some("parent-1".to_owned()),
    }));

    let sent = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.send_receipt.as_ref().is_some_and(|receipt| {
            receipt.request_id == 1 && receipt.state == TwitchSendReceiptState::Accepted
        }) && snapshot.messages.iter().any(|message| {
            message.id == "sent-message-1" && message.send_state == TwitchSendState::Received
        })
    });
    assert_eq!(sent.messages.len(), 1);
    assert_eq!(
        harness.http.lock().expect("HTTP state").sends,
        [(
            "100".to_owned(),
            "hello chat".to_owned(),
            Some("parent-1".to_owned()),
        )]
    );

    harness.incoming[0]
        .send(Ok(chat("delivery-echo", "sent-message-1", "hello chat")))
        .unwrap();
    let echoed = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.messages.len() == 1 && snapshot.messages[0].display_name == "Alice"
    });
    assert_eq!(echoed.messages[0].id, "sent-message-1");
}

#[test]
fn twitch_rejected_send_fails_once_without_retry() {
    let harness = harness(1);
    harness
        .http
        .lock()
        .expect("HTTP state")
        .send_results
        .push_back(Ok(ChatSendResult {
            message_id: None,
            is_sent: false,
        }));
    let joined = join_chat(&harness, 0);

    assert!(harness.client.try_send(TwitchCommand::SendMessage {
        request_id: 2,
        generation: joined.generation,
        channel: "warframe".to_owned(),
        text: "rejected".to_owned(),
        reply_to: None,
    }));

    let snapshot = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot
            .messages
            .iter()
            .any(|message| message.send_state == TwitchSendState::Failed)
    });
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(harness.http.lock().expect("HTTP state").sends.len(), 1);
}

#[test]
fn twitch_unauthorized_send_invalidates_the_session_and_closes_the_socket() {
    let harness = harness(1);
    harness
        .http
        .lock()
        .expect("HTTP state")
        .send_results
        .push_back(Err(HttpError::Status(401)));
    let joined = join_chat(&harness, 0);

    assert!(harness.client.try_send(TwitchCommand::SendMessage {
        request_id: 3,
        generation: joined.generation,
        channel: "warframe".to_owned(),
        text: "expired token".to_owned(),
        reply_to: None,
    }));

    let snapshot = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection
            == TwitchConnectionState::Failed(super::model::TwitchFailureCategory::Authentication)
    });
    assert_eq!(
        snapshot.send_receipt.as_ref().map(|receipt| receipt.state),
        Some(TwitchSendReceiptState::Rejected)
    );
    harness
        .dropped
        .recv_timeout(Duration::from_secs(1))
        .expect("unauthorized socket closed");
}

#[test]
fn twitch_eventsub_reconnect_uses_the_validated_url_without_resubscribing() {
    let harness = harness(2);
    join_chat(&harness, 0);
    harness.incoming[0]
        .send(Ok(
            r#"{"metadata":{"message_id":"reconnect-1","message_type":"session_reconnect"},"payload":{"session":{"id":"session-2","reconnect_url":"wss://eventsub.wss.twitch.tv/ws?reconnect=opaque"}}}"#.to_owned(),
        ))
        .unwrap();
    assert_eq!(
        harness
            .connects
            .recv_timeout(Duration::from_secs(1))
            .expect("replacement connection"),
        "wss://eventsub.wss.twitch.tv/ws?reconnect=opaque"
    );
    harness.incoming[1]
        .send(Ok(welcome("welcome-2", "session-2")))
        .unwrap();
    wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == TwitchConnectionState::Joined
    });
    assert_eq!(
        harness.http.lock().expect("HTTP state").subscriptions.len(),
        3
    );
}

#[test]
fn twitch_channel_switch_clears_transient_chat_and_creates_new_subscriptions() {
    let harness = harness(2);
    let first = join_chat(&harness, 0);
    harness.incoming[0]
        .send(Ok(chat("delivery-1", "message-1", "old channel")))
        .unwrap();
    wait_for_snapshot(&harness.client, |snapshot| snapshot.messages.len() == 1);

    let mut next_gate = open_gate();
    next_gate.channel = Some("playervox".to_owned());
    harness.client.set_gate(next_gate);
    assert_eq!(
        harness
            .connects
            .recv_timeout(Duration::from_secs(1))
            .expect("new EventSub connection"),
        INITIAL_EVENTSUB_URL
    );
    harness.incoming[1]
        .send(Ok(welcome("welcome-2", "session-2")))
        .unwrap();
    let switched = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == TwitchConnectionState::Joined
            && snapshot.channel.as_deref() == Some("playervox")
    });

    assert!(switched.messages.is_empty());
    assert!(switched.generation > first.generation);
    let state = harness.http.lock().expect("HTTP state");
    assert_eq!(state.resolved_logins, ["warframe", "playervox"]);
    assert_eq!(state.subscriptions.len(), 6);
}

#[test]
fn twitch_gate_close_drops_the_socket_and_fails_closed() {
    let harness = harness(1);
    join_chat(&harness, 0);

    harness.client.set_gate(TwitchGate::default());
    harness
        .dropped
        .recv_timeout(Duration::from_secs(1))
        .expect("socket closed");
    let snapshot = wait_for_snapshot(&harness.client, |snapshot| {
        snapshot.connection == TwitchConnectionState::Inert
    });
    assert!(snapshot.channel.is_none());
}

#[test]
fn twitch_client_drop_closes_a_live_socket() {
    let harness = harness(1);
    join_chat(&harness, 0);
    let Harness {
        client,
        incoming,
        connects,
        dropped,
        http,
    } = harness;
    let _keep_backend_alive = (incoming, connects, http);

    drop(client);

    dropped
        .recv_timeout(Duration::from_secs(1))
        .expect("shutdown closes the live socket");
}

#[test]
fn twitch_command_channel_is_bounded() {
    let (commands, _receiver) = command_channel();
    let accepted = (0..64)
        .filter(|_| commands.try_send(TwitchCommand::Reconnect).is_ok())
        .count();
    assert_eq!(accepted, 32);
}
