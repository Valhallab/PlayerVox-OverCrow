use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{
    auth::{AuthError, AuthMachine, AuthPresentation, AuthTick},
    credentials::{
        CredentialStore, CredentialStoreError, TwitchCredentials, decode_credentials,
        encode_credentials,
    },
    http::{
        ChatSendResult, ChatSubscription, DeviceCodeGrant, HttpError, TokenPoll, TokenResponse,
        TokenValidation, TwitchHttp, TwitchUser, validate_verification_uri,
    },
    model::TwitchFailureCategory,
};

const CLIENT_ID: &str = "public-client-id";

#[derive(Default)]
struct StoreState {
    credential: Option<TwitchCredentials>,
    save_count: usize,
    delete_count: usize,
    fail: bool,
}

#[derive(Clone, Default)]
struct FakeStore(Arc<Mutex<StoreState>>);

impl CredentialStore for FakeStore {
    fn load(&self) -> Result<Option<TwitchCredentials>, CredentialStoreError> {
        let state = self.0.lock().unwrap();
        if state.fail {
            return Err(CredentialStoreError::Unavailable);
        }
        Ok(state.credential.as_ref().map(TwitchCredentials::duplicate))
    }

    fn save(&self, credentials: &TwitchCredentials) -> Result<(), CredentialStoreError> {
        let mut state = self.0.lock().unwrap();
        if state.fail {
            return Err(CredentialStoreError::Unavailable);
        }
        state.credential = Some(credentials.duplicate());
        state.save_count += 1;
        Ok(())
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        let mut state = self.0.lock().unwrap();
        state.delete_count += 1;
        if state.fail {
            return Err(CredentialStoreError::Unavailable);
        }
        state.credential = None;
        Ok(())
    }
}

struct FakeHttp {
    grant: Result<DeviceCodeGrant, HttpError>,
    polls: VecDeque<Result<TokenPoll, HttpError>>,
    validations: VecDeque<Result<TokenValidation, HttpError>>,
    refresh: Option<Result<TokenResponse, HttpError>>,
    revoke: Result<(), HttpError>,
    poll_count: usize,
    validate_count: usize,
}

impl Default for FakeHttp {
    fn default() -> Self {
        Self {
            grant: Ok(DeviceCodeGrant {
                device_code: "device-secret".to_owned(),
                user_code: "ABCD-EFGH".to_owned(),
                verification_uri: "https://www.twitch.tv/activate".to_owned(),
                expires_in_secs: 600,
                interval_secs: 5,
            }),
            polls: VecDeque::new(),
            validations: VecDeque::new(),
            refresh: None,
            revoke: Ok(()),
            poll_count: 0,
            validate_count: 0,
        }
    }
}

impl TwitchHttp for FakeHttp {
    fn begin_device_authorization(
        &mut self,
        _client_id: &str,
    ) -> Result<DeviceCodeGrant, HttpError> {
        self.grant.clone()
    }

    fn poll_device_token(
        &mut self,
        _client_id: &str,
        _device_code: &str,
    ) -> Result<TokenPoll, HttpError> {
        self.poll_count += 1;
        self.polls.pop_front().unwrap_or(Ok(TokenPoll::Pending))
    }

    fn refresh_token(
        &mut self,
        _client_id: &str,
        _refresh_token: &str,
    ) -> Result<TokenResponse, HttpError> {
        self.refresh
            .take()
            .unwrap_or(Err(HttpError::ProviderResponse))
    }

    fn validate_token(&mut self, _access_token: &str) -> Result<TokenValidation, HttpError> {
        self.validate_count += 1;
        self.validations
            .pop_front()
            .unwrap_or(Err(HttpError::ProviderResponse))
    }

    fn revoke_token(&mut self, _client_id: &str, _access_token: &str) -> Result<(), HttpError> {
        self.revoke
    }

    fn resolve_channel(
        &mut self,
        _client_id: &str,
        _access_token: &str,
        _login: &str,
    ) -> Result<TwitchUser, HttpError> {
        Err(HttpError::InvalidRequest)
    }

    fn create_chat_subscription(
        &mut self,
        _client_id: &str,
        _access_token: &str,
        _session_id: &str,
        _broadcaster_user_id: &str,
        _user_id: &str,
        _subscription: ChatSubscription,
    ) -> Result<(), HttpError> {
        Err(HttpError::InvalidRequest)
    }

    fn send_chat_message(
        &mut self,
        _client_id: &str,
        _access_token: &str,
        _broadcaster_user_id: &str,
        _sender_id: &str,
        _message: &str,
        _reply_parent_message_id: Option<&str>,
    ) -> Result<ChatSendResult, HttpError> {
        Err(HttpError::InvalidRequest)
    }
}

fn token() -> TokenResponse {
    TokenResponse {
        access_token: "access-secret".to_owned(),
        refresh_token: "refresh-secret".to_owned(),
        expires_in_secs: 7_200,
        scopes: vec!["user:read:chat".to_owned(), "user:write:chat".to_owned()],
    }
}

fn validation() -> TokenValidation {
    TokenValidation {
        client_id: CLIENT_ID.to_owned(),
        login: "player_vox".to_owned(),
        user_id: "42".to_owned(),
        scopes: vec!["user:write:chat".to_owned(), "user:read:chat".to_owned()],
        expires_in_secs: 7_200,
    }
}

#[test]
fn twitch_credentials_round_trip_without_debugging_secrets() {
    let credentials = TwitchCredentials::new("access", "refresh", 1234).unwrap();
    let encoded = encode_credentials(&credentials).unwrap();
    let decoded = decode_credentials(&encoded).unwrap();

    assert_eq!(decoded.access_token(), "access");
    assert_eq!(decoded.refresh_token(), "refresh");
    assert_eq!(decoded.expires_at_unix_secs(), 1234);
    let debug = format!("{decoded:?}");
    assert!(!debug.contains("access"));
    assert!(!debug.contains("refresh"));
}

#[test]
fn twitch_http_url_policy_accepts_only_exact_https_authorities() {
    assert!(validate_verification_uri("https://www.twitch.tv/activate").is_ok());
    // Live Twitch Device Code responses use a single device-code query pair.
    // Observed live Twitch Device Code payload shape (2026-07).
    assert!(
        validate_verification_uri("https://www.twitch.tv/activate?device-code=CCYRGWNL").is_ok()
    );
    assert!(
        validate_verification_uri(
            "https://www.twitch.tv/activate?public=true&device-code=ABCDEFGH"
        )
        .is_ok()
    );
    for invalid in [
        "http://www.twitch.tv/activate",
        "https://user@www.twitch.tv/activate",
        "https://www.twitch.tv:444/activate",
        "https://evil.example/activate",
        "https://www.twitch.tv/other",
        "https://www.twitch.tv/activate?next=https://evil.example",
        "https://www.twitch.tv/activate?public=false&device-code=ABCDEFGH",
        "https://www.twitch.tv/activate?public=true&device-code=A%2FB",
        "https://www.twitch.tv/activate?device-code=CCYRGWNL&next=https://evil.example",
        "https://www.twitch.tv/activate?device-code=FIRST&device-code=SECOND",
        "https://www.twitch.tv/activate?device-code=",
    ] {
        assert!(validate_verification_uri(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn twitch_auth_device_flow_honors_poll_interval_and_connects() {
    let start = Instant::now();
    let store = FakeStore::default();
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp::default();
    http.polls.push_back(Ok(TokenPoll::Pending));
    http.polls.push_back(Ok(TokenPoll::Authorized(token())));
    http.validations.push_back(Ok(validation()));

    auth.begin(&mut http, start).unwrap();
    assert!(matches!(
        auth.presentation(),
        AuthPresentation::AwaitingUser { user_code, .. } if user_code == "ABCD-EFGH"
    ));
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(4), 1_000),
        AuthTick::Idle
    );
    assert_eq!(http.poll_count, 0);
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(5), 1_005),
        AuthTick::Changed
    );
    assert_eq!(http.poll_count, 1);
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(10), 1_010),
        AuthTick::Connected
    );
    assert_eq!(http.poll_count, 2);
    assert!(matches!(
        auth.presentation(),
        AuthPresentation::Connected { login, .. } if login == "player_vox"
    ));
    assert_eq!(auth.authenticated_user_id(), Some("42"));
    assert_eq!(store.0.lock().unwrap().save_count, 1);
}

#[test]
fn twitch_auth_cancel_and_expiry_clear_device_secrets() {
    let start = Instant::now();
    let store = FakeStore::default();
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store));
    let mut http = FakeHttp::default();
    auth.begin(&mut http, start).unwrap();
    auth.cancel();
    assert_eq!(auth.presentation(), &AuthPresentation::Disconnected);

    http.grant.as_mut().unwrap().expires_in_secs = 5;
    auth.begin(&mut http, start).unwrap();
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(6), 1_006),
        AuthTick::Changed
    );
    assert_eq!(
        auth.presentation(),
        &AuthPresentation::Failed(TwitchFailureCategory::AuthorizationExpired)
    );
}

#[test]
fn twitch_auth_rejects_wrong_client_or_missing_scopes() {
    for invalid in [
        TokenValidation {
            client_id: "other-client".to_owned(),
            ..validation()
        },
        TokenValidation {
            scopes: vec!["chat:read".to_owned()],
            ..validation()
        },
        TokenValidation {
            scopes: vec![
                "chat:read".to_owned(),
                "chat:edit".to_owned(),
                "user:read:email".to_owned(),
            ],
            ..validation()
        },
        TokenValidation {
            user_id: "not-a-numeric-id".to_owned(),
            ..validation()
        },
        TokenValidation {
            expires_in_secs: 31_536_001,
            ..validation()
        },
    ] {
        let start = Instant::now();
        let store = FakeStore::default();
        let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store));
        let mut http = FakeHttp::default();
        http.polls.push_back(Ok(TokenPoll::Authorized(token())));
        http.validations.push_back(Ok(invalid));
        auth.begin(&mut http, start).unwrap();

        assert_eq!(
            auth.tick(&mut http, start + Duration::from_secs(5), 1_005),
            AuthTick::Changed
        );
        assert_eq!(
            auth.presentation(),
            &AuthPresentation::Failed(TwitchFailureCategory::Authentication)
        );
        assert!(!auth.has_credentials());
    }
}

#[test]
fn transient_restore_failure_retains_credentials_and_retries_inertly() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("access", "refresh", 10_000).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp::default();
    http.validations.push_back(Err(HttpError::Transport));
    http.validations.push_back(Ok(validation()));
    let start = Instant::now();

    assert_eq!(auth.restore(&mut http, start, 1_000), AuthTick::Changed);
    assert!(auth.has_credentials());
    assert!(!auth.can_connect());
    assert_eq!(store.0.lock().unwrap().delete_count, 0);
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(29), 1_029),
        AuthTick::Idle
    );
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(30), 1_030),
        AuthTick::Changed
    );
    assert!(auth.can_connect());
    assert_eq!(store.0.lock().unwrap().delete_count, 0);
}

#[test]
fn transient_credential_store_failure_retries_restore_without_restart() {
    let store = FakeStore::default();
    store.0.lock().unwrap().fail = true;
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp::default();
    let start = Instant::now();

    assert_eq!(auth.restore(&mut http, start, 1_000), AuthTick::Changed);
    assert_eq!(
        auth.next_deadline(start, 1_000),
        start.checked_add(Duration::from_secs(30))
    );

    {
        let mut stored = store.0.lock().unwrap();
        stored.fail = false;
        stored.credential = Some(TwitchCredentials::new("access", "refresh", 10_000).unwrap());
    }
    http.validations.push_back(Ok(validation()));

    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(30), 1_030),
        AuthTick::Connected
    );
    assert!(auth.can_connect());
}

#[test]
fn unauthorized_restore_deletes_conclusively_invalid_credentials() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("access", "refresh", 10_000).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp::default();
    http.validations.push_back(Err(HttpError::Status(401)));

    assert_eq!(
        auth.restore(&mut http, Instant::now(), 1_000),
        AuthTick::Changed
    );
    assert!(!auth.has_credentials());
    assert_eq!(store.0.lock().unwrap().delete_count, 1);
}

#[test]
fn transient_refresh_failure_retains_credentials_without_reconnecting() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("access", "refresh", 1_100).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp::default();
    http.validations.push_back(Ok(TokenValidation {
        expires_in_secs: 100,
        ..validation()
    }));
    http.refresh = Some(Err(HttpError::Timeout));
    let start = Instant::now();
    assert_eq!(auth.restore(&mut http, start, 1_000), AuthTick::Connected);

    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(1), 1_001),
        AuthTick::Changed
    );
    assert!(auth.has_credentials());
    assert!(!auth.can_connect());
    assert_eq!(store.0.lock().unwrap().delete_count, 0);
}

#[test]
fn twitch_auth_restores_and_revalidates_credentials_hourly() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("access", "refresh", 10_000).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store));
    let mut http = FakeHttp::default();
    http.validations.push_back(Ok(validation()));
    http.validations.push_back(Ok(validation()));
    let start = Instant::now();

    assert_eq!(auth.restore(&mut http, start, 1_000), AuthTick::Connected);
    assert_eq!(http.validate_count, 1);
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(3_599), 4_599),
        AuthTick::Idle
    );
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(3_600), 4_600),
        AuthTick::Changed
    );
    assert_eq!(http.validate_count, 2);
}

#[test]
fn twitch_auth_disconnect_deletes_local_credentials_when_revocation_fails() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("access", "refresh", 10_000).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp {
        revoke: Err(HttpError::Transport),
        validations: VecDeque::from([Ok(validation())]),
        ..FakeHttp::default()
    };
    let start = Instant::now();
    auth.restore(&mut http, start, 1_000);

    assert!(auth.disconnect(&mut http).is_err());
    assert_eq!(auth.presentation(), &AuthPresentation::Disconnected);
    assert!(!auth.has_credentials());
    assert_eq!(store.0.lock().unwrap().delete_count, 1);
}

#[test]
fn twitch_auth_persists_rotated_tokens_and_validates_the_new_session() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("old-access", "old-refresh", 1_100).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp::default();
    http.validations.push_back(Ok(TokenValidation {
        expires_in_secs: 100,
        ..validation()
    }));
    http.refresh = Some(Ok(TokenResponse {
        access_token: "new-access".to_owned(),
        refresh_token: "new-refresh".to_owned(),
        ..token()
    }));
    http.validations.push_back(Ok(validation()));
    let start = Instant::now();
    assert_eq!(auth.restore(&mut http, start, 1_000), AuthTick::Connected);

    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(1), 1_001),
        AuthTick::Changed
    );
    let stored = store.0.lock().unwrap();
    let credentials = stored.credential.as_ref().unwrap();
    assert_eq!(credentials.access_token(), "new-access");
    assert_eq!(credentials.refresh_token(), "new-refresh");
    assert_eq!(http.validate_count, 2);
}

#[test]
fn rotated_tokens_are_persisted_before_transient_validation_and_reused_after_retry() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("old-access", "old-refresh", 1_100).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp::default();
    http.validations.push_back(Ok(TokenValidation {
        expires_in_secs: 100,
        ..validation()
    }));
    http.refresh = Some(Ok(TokenResponse {
        access_token: "new-access".to_owned(),
        refresh_token: "new-refresh".to_owned(),
        ..token()
    }));
    http.validations.push_back(Err(HttpError::Timeout));
    http.validations.push_back(Ok(validation()));
    let start = Instant::now();

    assert_eq!(auth.restore(&mut http, start, 1_000), AuthTick::Connected);
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(1), 1_001),
        AuthTick::Changed
    );
    assert!(auth.has_credentials());
    assert!(auth.credentials_persisted());
    {
        let stored = store.0.lock().unwrap();
        let credentials = stored.credential.as_ref().unwrap();
        assert_eq!(credentials.access_token(), "new-access");
        assert_eq!(credentials.refresh_token(), "new-refresh");
    }

    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(31), 1_031),
        AuthTick::Changed
    );
    assert!(auth.can_connect());
    assert!(auth.credentials_persisted());
    let stored = store.0.lock().unwrap();
    let credentials = stored.credential.as_ref().unwrap();
    assert_eq!(credentials.access_token(), "new-access");
    assert_eq!(credentials.refresh_token(), "new-refresh");
}

#[test]
fn failed_credential_deletion_keeps_the_existing_session_visible() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("access", "refresh", 10_000).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp {
        validations: VecDeque::from([Ok(validation())]),
        ..FakeHttp::default()
    };
    assert_eq!(
        auth.restore(&mut http, Instant::now(), 1_000),
        AuthTick::Connected
    );
    store.0.lock().unwrap().fail = true;

    assert!(matches!(
        auth.disconnect(&mut http),
        Err(AuthError::Credentials(CredentialStoreError::Unavailable))
    ));
    assert_eq!(
        auth.presentation(),
        &AuthPresentation::Failed(TwitchFailureCategory::CredentialStore)
    );
    assert!(auth.has_credentials());
    assert!(auth.credentials_persisted());
    assert_eq!(store.0.lock().unwrap().delete_count, 1);
}

#[test]
fn authentication_cannot_replace_an_existing_session() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("access", "refresh", 10_000).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store));
    let mut http = FakeHttp {
        validations: VecDeque::from([Ok(validation())]),
        ..FakeHttp::default()
    };
    assert_eq!(
        auth.restore(&mut http, Instant::now(), 1_000),
        AuthTick::Connected
    );

    assert_eq!(
        auth.begin(&mut http, Instant::now()),
        Err(AuthError::SessionExists)
    );
    assert!(auth.has_credentials());
    assert!(auth.credentials_persisted());
}

#[test]
fn twitch_auth_keeps_a_successful_session_in_memory_when_keyring_is_unavailable() {
    let start = Instant::now();
    let store = FakeStore::default();
    store.0.lock().unwrap().fail = true;
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store));
    let mut http = FakeHttp::default();
    http.polls.push_back(Ok(TokenPoll::Authorized(token())));
    http.validations.push_back(Ok(validation()));

    auth.begin(&mut http, start).unwrap();
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(5), 1_005),
        AuthTick::Connected
    );
    assert!(auth.has_credentials());
    assert!(!auth.credentials_persisted());
}

#[test]
fn disconnect_remains_pending_when_keyring_cleanup_cannot_be_confirmed() {
    let start = Instant::now();
    let store = FakeStore::default();
    store.0.lock().unwrap().fail = true;
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store));
    let mut http = FakeHttp::default();
    http.polls.push_back(Ok(TokenPoll::Authorized(token())));
    http.validations.push_back(Ok(validation()));

    auth.begin(&mut http, start).unwrap();
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(5), 1_005),
        AuthTick::Connected
    );

    assert!(auth.disconnect_local().is_err());
    assert_eq!(
        auth.presentation(),
        &AuthPresentation::Failed(TwitchFailureCategory::CredentialStore)
    );
    assert!(auth.has_credentials());
    assert!(auth.credentials_available());
    assert!(!auth.credentials_persisted());
}

#[test]
fn failed_refresh_overwrite_and_delete_never_claim_a_durable_disconnect() {
    let store = FakeStore::default();
    store.0.lock().unwrap().credential =
        Some(TwitchCredentials::new("old-access", "old-refresh", 1_100).unwrap());
    let mut auth = AuthMachine::new(CLIENT_ID, Arc::new(store.clone()));
    let mut http = FakeHttp::default();
    http.validations.push_back(Ok(TokenValidation {
        expires_in_secs: 100,
        ..validation()
    }));
    http.refresh = Some(Ok(TokenResponse {
        access_token: "new-access".to_owned(),
        refresh_token: "new-refresh".to_owned(),
        ..token()
    }));
    http.validations.push_back(Err(HttpError::Timeout));
    let start = Instant::now();
    assert_eq!(auth.restore(&mut http, start, 1_000), AuthTick::Connected);

    store.0.lock().unwrap().fail = true;
    assert_eq!(
        auth.tick(&mut http, start + Duration::from_secs(1), 1_001),
        AuthTick::Changed
    );
    assert!(!auth.credentials_persisted());
    assert!(auth.credentials_available());

    assert!(auth.disconnect_local().is_err());
    assert_eq!(
        auth.presentation(),
        &AuthPresentation::Failed(TwitchFailureCategory::CredentialStore)
    );
    assert!(auth.has_credentials());
    assert!(auth.credentials_available());
    let stored = store.0.lock().unwrap();
    let credentials = stored.credential.as_ref().unwrap();
    assert_eq!(credentials.access_token(), "old-access");
    assert_eq!(credentials.refresh_token(), "old-refresh");
}
