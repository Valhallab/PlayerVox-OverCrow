use std::{
    error::Error,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use zeroize::Zeroizing;

use super::{
    credentials::{CredentialStore, CredentialStoreError, TwitchCredentials},
    http::{
        HttpError, TokenPoll, TokenResponse, TokenValidation, TwitchHttp, validate_verification_uri,
    },
    model::TwitchFailureCategory,
};

const VALIDATION_INTERVAL: Duration = Duration::from_secs(60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(30);
const REFRESH_MARGIN_SECS: u64 = 5 * 60;
const REQUIRED_SCOPES: [&str; 2] = ["user:read:chat", "user:write:chat"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthPresentation {
    Disconnected,
    AwaitingUser {
        user_code: String,
        verification_uri: String,
        expires_at: Instant,
    },
    Connected {
        login: String,
        user_id: String,
    },
    Failed(TwitchFailureCategory),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthTick {
    Idle,
    Changed,
    Connected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthError {
    ClientUnavailable,
    SessionExists,
    InvalidCredentials,
    Provider(HttpError),
    Credentials(CredentialStoreError),
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClientUnavailable => formatter.write_str("Twitch client is not configured"),
            Self::SessionExists => formatter.write_str("a Twitch session already exists"),
            Self::InvalidCredentials => formatter.write_str("Twitch credentials are invalid"),
            Self::Provider(error) => error.fmt(formatter),
            Self::Credentials(error) => error.fmt(formatter),
        }
    }
}

impl Error for AuthError {}

impl From<HttpError> for AuthError {
    fn from(error: HttpError) -> Self {
        Self::Provider(error)
    }
}

impl From<CredentialStoreError> for AuthError {
    fn from(error: CredentialStoreError) -> Self {
        Self::Credentials(error)
    }
}

struct PendingAuthorization {
    device_code: Zeroizing<String>,
    expires_at: Instant,
    interval: Duration,
    next_poll: Instant,
}

pub struct AuthMachine {
    client_id: String,
    store: Arc<dyn CredentialStore>,
    credentials: Option<TwitchCredentials>,
    pending: Option<PendingAuthorization>,
    presentation: AuthPresentation,
    last_validated_at: Option<Instant>,
    retry_at: Option<Instant>,
    credentials_persisted: bool,
    credentials_need_persist: bool,
    store_may_contain_credentials: bool,
}

impl AuthMachine {
    pub fn new(client_id: impl Into<String>, store: Arc<dyn CredentialStore>) -> Self {
        Self {
            client_id: client_id.into(),
            store,
            credentials: None,
            pending: None,
            presentation: AuthPresentation::Disconnected,
            last_validated_at: None,
            retry_at: None,
            credentials_persisted: false,
            credentials_need_persist: false,
            store_may_contain_credentials: false,
        }
    }

    pub fn presentation(&self) -> &AuthPresentation {
        &self.presentation
    }

    pub fn is_configured(&self) -> bool {
        !self.client_id.is_empty()
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn has_credentials(&self) -> bool {
        self.credentials.is_some()
    }

    pub fn credentials_available(&self) -> bool {
        self.credentials.is_some() || self.store_may_contain_credentials
    }

    pub fn can_connect(&self) -> bool {
        self.credentials.is_some()
            && matches!(self.presentation, AuthPresentation::Connected { .. })
    }

    pub fn credentials_persisted(&self) -> bool {
        self.credentials_persisted
    }

    pub fn access_token(&self) -> Option<&str> {
        self.credentials
            .as_ref()
            .map(TwitchCredentials::access_token)
    }

    pub fn authenticated_login(&self) -> Option<&str> {
        match &self.presentation {
            AuthPresentation::Connected { login, .. } => Some(login),
            _ => None,
        }
    }

    pub fn authenticated_user_id(&self) -> Option<&str> {
        match &self.presentation {
            AuthPresentation::Connected { user_id, .. } => Some(user_id),
            _ => None,
        }
    }

    pub fn next_deadline(&self, now: Instant, now_unix_secs: u64) -> Option<Instant> {
        if let Some(pending) = &self.pending {
            return Some(pending.next_poll.min(pending.expires_at));
        }
        if let Some(retry_at) = self.retry_at {
            return Some(retry_at);
        }
        let credentials = self.credentials.as_ref()?;
        let validation_deadline = self
            .last_validated_at
            .and_then(|validated| validated.checked_add(VALIDATION_INTERVAL))
            .unwrap_or(now);
        let seconds_until_refresh = credentials
            .expires_at_unix_secs()
            .saturating_sub(now_unix_secs.saturating_add(REFRESH_MARGIN_SECS));
        let refresh_deadline = now
            .checked_add(Duration::from_secs(seconds_until_refresh))
            .unwrap_or(validation_deadline);
        Some(validation_deadline.min(refresh_deadline))
    }

    pub fn begin(&mut self, http: &mut dyn TwitchHttp, now: Instant) -> Result<(), AuthError> {
        if self.client_id.is_empty() {
            return Err(AuthError::ClientUnavailable);
        }
        if self.credentials.is_some() || self.store_may_contain_credentials {
            return Err(AuthError::SessionExists);
        }
        let grant = match http.begin_device_authorization(&self.client_id) {
            Ok(grant) => grant,
            Err(error) => {
                self.presentation = AuthPresentation::Failed(begin_failure_category(&error));
                return Err(error.into());
            }
        };
        if let Err(error) = validate_verification_uri(&grant.verification_uri) {
            self.presentation = AuthPresentation::Failed(begin_failure_category(&error));
            return Err(error.into());
        }
        let expires_at = match now.checked_add(Duration::from_secs(grant.expires_in_secs)) {
            Some(deadline) => deadline,
            None => {
                self.presentation =
                    AuthPresentation::Failed(TwitchFailureCategory::ProviderResponse);
                return Err(HttpError::ProviderResponse.into());
            }
        };
        let interval = Duration::from_secs(grant.interval_secs.clamp(1, 60));
        let next_poll = match now.checked_add(interval) {
            Some(deadline) => deadline,
            None => {
                self.presentation =
                    AuthPresentation::Failed(TwitchFailureCategory::ProviderResponse);
                return Err(HttpError::ProviderResponse.into());
            }
        };
        self.credentials = None;
        self.pending = Some(PendingAuthorization {
            device_code: Zeroizing::new(grant.device_code),
            expires_at,
            interval,
            next_poll,
        });
        self.presentation = AuthPresentation::AwaitingUser {
            user_code: grant.user_code,
            verification_uri: grant.verification_uri,
            expires_at,
        };
        self.last_validated_at = None;
        self.retry_at = None;
        self.credentials_persisted = false;
        self.credentials_need_persist = false;
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.pending = None;
        self.presentation = AuthPresentation::Disconnected;
    }

    pub fn cancel_pending(&mut self) {
        if self.pending.take().is_some() {
            self.presentation = AuthPresentation::Disconnected;
        }
    }

    pub fn invalidate(&mut self, category: TwitchFailureCategory) {
        self.clear_invalid_credentials(category);
    }

    pub fn restore(
        &mut self,
        http: &mut dyn TwitchHttp,
        now: Instant,
        now_unix_secs: u64,
    ) -> AuthTick {
        let credentials = match self.store.load() {
            Ok(Some(credentials)) => credentials,
            Ok(None) => {
                self.store_may_contain_credentials = false;
                self.retry_at = None;
                self.presentation = AuthPresentation::Disconnected;
                return AuthTick::Idle;
            }
            Err(_) => {
                self.store_may_contain_credentials = true;
                self.retry_at = now.checked_add(RETRY_INTERVAL);
                self.presentation =
                    AuthPresentation::Failed(TwitchFailureCategory::CredentialStore);
                return AuthTick::Changed;
            }
        };
        self.credentials = Some(credentials);
        self.credentials_persisted = true;
        self.credentials_need_persist = false;
        self.store_may_contain_credentials = true;
        self.retry_at = None;
        match self.validate_current(http, now, now_unix_secs) {
            Ok(()) => AuthTick::Connected,
            Err(error) => {
                self.handle_credential_failure(error, now);
                AuthTick::Changed
            }
        }
    }

    pub fn tick(
        &mut self,
        http: &mut dyn TwitchHttp,
        now: Instant,
        now_unix_secs: u64,
    ) -> AuthTick {
        if self.pending.is_some() {
            return self.tick_pending(http, now, now_unix_secs);
        }
        if self.credentials.is_none() && self.store_may_contain_credentials {
            if self.retry_at.is_some_and(|retry_at| now < retry_at) {
                return AuthTick::Idle;
            }
            return self.restore(http, now, now_unix_secs);
        }
        if self.credentials.is_none() {
            return AuthTick::Idle;
        }
        if self.retry_at.is_some_and(|retry_at| now < retry_at) {
            return AuthTick::Idle;
        }

        let expires_at = self
            .credentials
            .as_ref()
            .map(TwitchCredentials::expires_at_unix_secs)
            .unwrap_or_default();
        if expires_at <= now_unix_secs.saturating_add(REFRESH_MARGIN_SECS) {
            if let Err(error) = self.refresh(http, now, now_unix_secs) {
                self.handle_credential_failure(error, now);
            }
            return AuthTick::Changed;
        }

        let validation_due = self
            .last_validated_at
            .and_then(|validated| now.checked_duration_since(validated))
            .is_none_or(|elapsed| elapsed >= VALIDATION_INTERVAL);
        if !validation_due {
            return AuthTick::Idle;
        }
        match self.validate_current(http, now, now_unix_secs) {
            Ok(()) => AuthTick::Changed,
            Err(error) => {
                self.handle_credential_failure(error, now);
                AuthTick::Changed
            }
        }
    }

    pub fn disconnect(&mut self, http: &mut dyn TwitchHttp) -> Result<(), AuthError> {
        let revoke_result = self
            .credentials
            .as_ref()
            .map(|credentials| {
                http.revoke_token(&self.client_id, credentials.access_token())
                    .map_err(AuthError::from)
            })
            .transpose();
        self.disconnect_local()?;
        revoke_result.map(|_| ())
    }

    pub fn disconnect_local(&mut self) -> Result<(), AuthError> {
        self.pending = None;
        if let Err(error) = self.store.delete().map_err(AuthError::from) {
            self.store_may_contain_credentials = true;
            self.presentation = AuthPresentation::Failed(TwitchFailureCategory::CredentialStore);
            self.last_validated_at = None;
            self.retry_at = None;
            return Err(error);
        }
        self.clear_local_session();
        Ok(())
    }

    fn clear_local_session(&mut self) {
        self.credentials = None;
        self.last_validated_at = None;
        self.retry_at = None;
        self.credentials_persisted = false;
        self.credentials_need_persist = false;
        self.store_may_contain_credentials = false;
        self.presentation = AuthPresentation::Disconnected;
    }

    fn tick_pending(
        &mut self,
        http: &mut dyn TwitchHttp,
        now: Instant,
        now_unix_secs: u64,
    ) -> AuthTick {
        let Some(pending) = self.pending.as_mut() else {
            return AuthTick::Idle;
        };
        if now >= pending.expires_at {
            self.pending = None;
            self.presentation =
                AuthPresentation::Failed(TwitchFailureCategory::AuthorizationExpired);
            return AuthTick::Changed;
        }
        if now < pending.next_poll {
            return AuthTick::Idle;
        }
        let poll = http.poll_device_token(&self.client_id, &pending.device_code);
        match poll {
            Ok(TokenPoll::Pending) => {
                pending.next_poll = now
                    .checked_add(pending.interval)
                    .unwrap_or(pending.expires_at);
                AuthTick::Changed
            }
            Ok(TokenPoll::SlowDown) => {
                pending.interval =
                    (pending.interval + Duration::from_secs(5)).min(Duration::from_secs(60));
                pending.next_poll = now
                    .checked_add(pending.interval)
                    .unwrap_or(pending.expires_at);
                AuthTick::Changed
            }
            Ok(TokenPoll::Denied) => {
                self.pending = None;
                self.presentation = AuthPresentation::Failed(TwitchFailureCategory::Authentication);
                AuthTick::Changed
            }
            Ok(TokenPoll::Expired) => {
                self.pending = None;
                self.presentation =
                    AuthPresentation::Failed(TwitchFailureCategory::AuthorizationExpired);
                AuthTick::Changed
            }
            Ok(TokenPoll::Authorized(token)) => {
                self.pending = None;
                match self.accept_token(http, token, now, now_unix_secs) {
                    Ok(()) => AuthTick::Connected,
                    Err(error) => {
                        self.handle_credential_failure(error, now);
                        AuthTick::Changed
                    }
                }
            }
            Err(_) => {
                pending.next_poll = now
                    .checked_add(pending.interval)
                    .unwrap_or(pending.expires_at);
                AuthTick::Changed
            }
        }
    }

    fn accept_token(
        &mut self,
        http: &mut dyn TwitchHttp,
        token: TokenResponse,
        now: Instant,
        now_unix_secs: u64,
    ) -> Result<(), AuthError> {
        if !has_required_scopes(&token.scopes) {
            return Err(AuthError::InvalidCredentials);
        }
        let expires_at = now_unix_secs
            .checked_add(token.expires_in_secs)
            .ok_or(HttpError::ProviderResponse)?;
        self.credentials = Some(TwitchCredentials::new(
            token.access_token,
            token.refresh_token,
            expires_at,
        )?);
        self.credentials_persisted = false;
        self.credentials_need_persist = true;
        self.validate_current(http, now, now_unix_secs)?;
        Ok(())
    }

    fn refresh(
        &mut self,
        http: &mut dyn TwitchHttp,
        now: Instant,
        now_unix_secs: u64,
    ) -> Result<(), AuthError> {
        let refresh_token = self
            .credentials
            .as_ref()
            .map(|credentials| Zeroizing::new(credentials.refresh_token().to_owned()))
            .ok_or(HttpError::ProviderResponse)?;
        let token = http
            .refresh_token(&self.client_id, &refresh_token)
            .map_err(map_refresh_error)?;
        if !has_required_scopes(&token.scopes) {
            return Err(AuthError::InvalidCredentials);
        }
        let expires_at = now_unix_secs
            .checked_add(token.expires_in_secs)
            .ok_or(HttpError::ProviderResponse)?;
        let credentials =
            TwitchCredentials::new(token.access_token, token.refresh_token, expires_at)?;
        self.credentials = Some(credentials);
        self.credentials_persisted = false;
        self.credentials_need_persist = true;
        // Twitch rotates device-flow refresh tokens. Store the replacement
        // before another network round-trip so a crash cannot revive the
        // consumed token. Restore will validate and remove an invalid value.
        self.persist_current_if_needed();
        self.validate_current(http, now, now_unix_secs)?;
        Ok(())
    }

    fn validate_current(
        &mut self,
        http: &mut dyn TwitchHttp,
        now: Instant,
        now_unix_secs: u64,
    ) -> Result<(), AuthError> {
        let access_token = self
            .credentials
            .as_ref()
            .map(|credentials| Zeroizing::new(credentials.access_token().to_owned()))
            .ok_or(HttpError::ProviderResponse)?;
        let validation = http
            .validate_token(&access_token)
            .map_err(map_validation_error)?;
        validate_identity(&self.client_id, &validation)?;
        if let Some(credentials) = self.credentials.as_mut() {
            credentials.set_expiry(
                now_unix_secs
                    .checked_add(validation.expires_in_secs)
                    .ok_or(HttpError::ProviderResponse)?,
            );
        }
        self.presentation = AuthPresentation::Connected {
            login: validation.login,
            user_id: validation.user_id,
        };
        self.last_validated_at = Some(now);
        self.retry_at = None;
        self.persist_current_if_needed();
        Ok(())
    }

    fn persist_current_if_needed(&mut self) {
        if !self.credentials_need_persist {
            return;
        }
        let saved = self
            .credentials
            .as_ref()
            .is_some_and(|credentials| self.store.save(credentials).is_ok());
        self.credentials_persisted = saved;
        self.credentials_need_persist = !saved;
        self.store_may_contain_credentials = true;
    }

    fn handle_credential_failure(&mut self, error: AuthError, now: Instant) {
        if error.is_conclusive() {
            self.clear_invalid_credentials(TwitchFailureCategory::Authentication);
            return;
        }
        self.last_validated_at = None;
        self.retry_at = now.checked_add(RETRY_INTERVAL);
        self.presentation = AuthPresentation::Failed(match error {
            AuthError::Credentials(_) => TwitchFailureCategory::CredentialStore,
            AuthError::Provider(HttpError::Timeout | HttpError::Transport) => {
                TwitchFailureCategory::Connection
            }
            _ => TwitchFailureCategory::ProviderResponse,
        });
    }

    fn clear_invalid_credentials(&mut self, category: TwitchFailureCategory) {
        self.credentials = None;
        self.pending = None;
        self.last_validated_at = None;
        self.retry_at = None;
        self.credentials_persisted = false;
        self.credentials_need_persist = false;
        let deleted = self.store.delete().is_ok();
        self.store_may_contain_credentials = !deleted;
        self.presentation = AuthPresentation::Failed(if deleted {
            category
        } else {
            TwitchFailureCategory::CredentialStore
        });
    }
}

fn begin_failure_category(error: &HttpError) -> TwitchFailureCategory {
    match error {
        HttpError::Timeout | HttpError::Transport => TwitchFailureCategory::Connection,
        HttpError::Status(429) => TwitchFailureCategory::RateLimited,
        _ => TwitchFailureCategory::ProviderResponse,
    }
}

fn validate_identity(
    expected_client_id: &str,
    validation: &TokenValidation,
) -> Result<(), AuthError> {
    if validation.client_id != expected_client_id
        || validation.login.is_empty()
        || validation.login.len() > 25
        || !validation
            .login
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        || validation.user_id.is_empty()
        || validation.user_id.len() > 32
        || !validation.user_id.bytes().all(|byte| byte.is_ascii_digit())
        || validation.expires_in_secs == 0
        || validation.expires_in_secs > 31_536_000
        || !has_required_scopes(&validation.scopes)
    {
        return Err(AuthError::InvalidCredentials);
    }
    Ok(())
}

fn has_required_scopes(scopes: &[String]) -> bool {
    scopes.len() == REQUIRED_SCOPES.len()
        && REQUIRED_SCOPES
            .iter()
            .all(|required| scopes.iter().any(|scope| scope == required))
}

fn map_validation_error(error: HttpError) -> AuthError {
    if matches!(error, HttpError::Status(401)) {
        AuthError::InvalidCredentials
    } else {
        AuthError::Provider(error)
    }
}

fn map_refresh_error(error: HttpError) -> AuthError {
    if matches!(error, HttpError::Status(400 | 401)) {
        AuthError::InvalidCredentials
    } else {
        AuthError::Provider(error)
    }
}

impl AuthError {
    fn is_conclusive(self) -> bool {
        matches!(self, Self::InvalidCredentials)
    }
}
