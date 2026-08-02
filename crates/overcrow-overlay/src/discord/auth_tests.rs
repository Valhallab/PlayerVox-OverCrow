use std::sync::{Arc, Mutex};

use super::{
    auth::{AuthError, AuthPersistence, DiscordAuth},
    credentials::{
        CredentialStore, CredentialStoreError, DiscordCredentials, decode_credentials,
        encode_credentials,
    },
    model::SensitiveValue,
    oauth::{DiscordOauth, OauthError, TokenResponse, parse_token_response},
};

#[derive(Default)]
struct StoreState {
    credentials: Option<DiscordCredentials>,
    fail_load: bool,
    fail_save: bool,
    fail_delete: bool,
}

#[derive(Default)]
struct FakeStore(Mutex<StoreState>);

impl CredentialStore for FakeStore {
    fn load(&self) -> Result<Option<DiscordCredentials>, CredentialStoreError> {
        let state = self.0.lock().unwrap();
        if state.fail_load {
            Err(CredentialStoreError::Unavailable)
        } else {
            Ok(state
                .credentials
                .as_ref()
                .map(DiscordCredentials::duplicate))
        }
    }

    fn save(&self, credentials: &DiscordCredentials) -> Result<(), CredentialStoreError> {
        let mut state = self.0.lock().unwrap();
        if state.fail_save {
            Err(CredentialStoreError::Unavailable)
        } else {
            state.credentials = Some(credentials.duplicate());
            Ok(())
        }
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        let mut state = self.0.lock().unwrap();
        if state.fail_delete {
            Err(CredentialStoreError::Unavailable)
        } else {
            state.credentials = None;
            Ok(())
        }
    }
}

#[derive(Default)]
struct FakeOauth {
    exchange: Option<Result<TokenResponse, OauthError>>,
    refresh: Option<Result<TokenResponse, OauthError>>,
    revoked: Vec<String>,
    refreshes: usize,
}

impl DiscordOauth for FakeOauth {
    fn exchange(&mut self, _code: &str) -> Result<TokenResponse, OauthError> {
        self.exchange.take().unwrap()
    }

    fn refresh(&mut self, _refresh_token: &str) -> Result<TokenResponse, OauthError> {
        self.refreshes += 1;
        self.refresh.take().unwrap()
    }

    fn revoke(&mut self, access_token: &str) -> Result<(), OauthError> {
        self.revoked.push(access_token.to_owned());
        Ok(())
    }
}

fn token(access: &str, refresh: &str, expires_in_secs: u64) -> TokenResponse {
    TokenResponse {
        access_token: access.to_owned(),
        refresh_token: refresh.to_owned(),
        expires_in_secs,
    }
}

#[test]
fn credentials_round_trip_without_exposing_secrets() {
    let credentials = DiscordCredentials::new("access-secret", "refresh-secret", 10_000).unwrap();

    let encoded = encode_credentials(&credentials).unwrap();
    let decoded = decode_credentials(&encoded).unwrap();

    assert_eq!(decoded.access_token(), "access-secret");
    assert_eq!(decoded.refresh_token(), "refresh-secret");
    assert_eq!(decoded.expires_at_unix_secs(), 10_000);
    let debug = format!("{decoded:?}");
    assert!(!debug.contains("access-secret"));
    assert!(!debug.contains("refresh-secret"));
}

#[test]
fn credential_decoder_rejects_unknown_versions_unknown_fields_and_oversize() {
    assert!(matches!(
        decode_credentials(br#"{"schema_version":2,"access_token":"a","refresh_token":"b","expires_at_unix_secs":1}"#),
        Err(CredentialStoreError::Corrupt)
    ));
    assert!(matches!(
        decode_credentials(br#"{"schema_version":1,"access_token":"a","refresh_token":"b","expires_at_unix_secs":1,"extra":true}"#),
        Err(CredentialStoreError::Corrupt)
    ));
    assert!(matches!(
        decode_credentials(&vec![b'x'; 12 * 1024 + 1]),
        Err(CredentialStoreError::Corrupt)
    ));
}

#[test]
fn authorization_keeps_credentials_in_memory_when_secret_service_save_fails() {
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().fail_save = true;
    let mut auth = DiscordAuth::new(store);
    let mut oauth = FakeOauth {
        exchange: Some(Ok(token("access", "refresh", 3_600))),
        ..FakeOauth::default()
    };

    let persistence = auth
        .authorize(
            SensitiveValue::new("authorization-code".to_owned()),
            &mut oauth,
            1_000,
        )
        .unwrap();

    assert_eq!(persistence, AuthPersistence::MemoryOnly);
    assert_eq!(auth.access_token(), Some("access"));
    assert!(!auth.credentials_persisted());
}

#[test]
fn expiring_credentials_refresh_once_and_rotate_the_persisted_pair() {
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("old-access", "old-refresh", 1_200).unwrap());
    let mut auth = DiscordAuth::new(store.clone());
    assert_eq!(auth.restore().unwrap(), AuthPersistence::Persisted);
    let mut oauth = FakeOauth {
        refresh: Some(Ok(token("new-access", "new-refresh", 7_200))),
        ..FakeOauth::default()
    };

    assert_eq!(
        auth.refresh_if_needed(&mut oauth, 1_000).unwrap(),
        AuthPersistence::Persisted
    );

    assert_eq!(oauth.refreshes, 1);
    assert_eq!(auth.access_token(), Some("new-access"));
    assert_eq!(
        store
            .0
            .lock()
            .unwrap()
            .credentials
            .as_ref()
            .unwrap()
            .refresh_token(),
        "new-refresh"
    );
}

#[test]
fn rejected_credentials_get_one_refresh_attempt_then_clear_on_unauthorized() {
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", 10_000).unwrap());
    let mut auth = DiscordAuth::new(store.clone());
    auth.restore().unwrap();
    let mut oauth = FakeOauth {
        refresh: Some(Err(OauthError::Unauthorized)),
        ..FakeOauth::default()
    };

    assert_eq!(
        auth.refresh_after_rejection(&mut oauth, 1_000),
        Err(AuthError::AuthorizationExpired)
    );

    assert_eq!(oauth.refreshes, 1);
    assert_eq!(auth.access_token(), None);
    assert!(store.0.lock().unwrap().credentials.is_none());
}

#[test]
fn sign_out_revokes_best_effort_and_always_clears_memory() {
    let store = Arc::new(FakeStore::default());
    store.0.lock().unwrap().credentials =
        Some(DiscordCredentials::new("access", "refresh", 10_000).unwrap());
    let mut auth = DiscordAuth::new(store.clone());
    auth.restore().unwrap();
    let mut oauth = FakeOauth::default();

    auth.sign_out(&mut oauth).unwrap();

    assert_eq!(oauth.revoked, ["access"]);
    assert_eq!(auth.access_token(), None);
    assert!(store.0.lock().unwrap().credentials.is_none());
}

#[test]
fn sign_out_keeps_an_actionable_session_when_secret_deletion_fails() {
    let store = Arc::new(FakeStore::default());
    {
        let mut state = store.0.lock().unwrap();
        state.credentials = Some(DiscordCredentials::new("access", "refresh", 10_000).unwrap());
        state.fail_delete = true;
    }
    let mut auth = DiscordAuth::new(store.clone());
    auth.restore().unwrap();
    let mut oauth = FakeOauth::default();

    assert_eq!(
        auth.sign_out(&mut oauth),
        Err(AuthError::CredentialStore(
            CredentialStoreError::Unavailable
        ))
    );

    assert_eq!(oauth.revoked, ["access"]);
    assert_eq!(auth.access_token(), Some("access"));
    assert!(auth.credentials_persisted());
    assert!(store.0.lock().unwrap().credentials.is_some());
}

#[test]
fn broker_token_response_is_strict_and_bounded() {
    let valid = br#"{"access_token":"access","refresh_token":"refresh","expires_in":604800}"#;
    assert_eq!(
        parse_token_response(valid).unwrap(),
        token("access", "refresh", 604_800)
    );
    assert_eq!(
        parse_token_response(br#"{"access_token":"a","refresh_token":"b","expires_in":0}"#),
        Err(OauthError::InvalidResponse)
    );
    assert_eq!(
        parse_token_response(
            br#"{"access_token":"a","refresh_token":"b","expires_in":1,"extra":true}"#
        ),
        Err(OauthError::InvalidResponse)
    );
}
