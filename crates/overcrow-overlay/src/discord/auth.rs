use std::{error::Error, fmt, sync::Arc};

use super::{
    credentials::{CredentialStore, CredentialStoreError, DiscordCredentials},
    model::SensitiveValue,
    oauth::{DiscordOauth, OauthError, TokenResponse},
};

const REFRESH_MARGIN_SECS: u64 = 5 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthPersistence {
    Persisted,
    MemoryOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthError {
    AuthorizationExpired,
    CredentialStore(CredentialStoreError),
    Oauth(OauthError),
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthorizationExpired => formatter.write_str("Discord authorization expired"),
            Self::CredentialStore(error) => error.fmt(formatter),
            Self::Oauth(error) => error.fmt(formatter),
        }
    }
}

impl Error for AuthError {}

impl From<CredentialStoreError> for AuthError {
    fn from(error: CredentialStoreError) -> Self {
        Self::CredentialStore(error)
    }
}

impl From<OauthError> for AuthError {
    fn from(error: OauthError) -> Self {
        Self::Oauth(error)
    }
}

pub struct DiscordAuth {
    store: Arc<dyn CredentialStore>,
    credentials: Option<DiscordCredentials>,
    persisted: bool,
}

impl DiscordAuth {
    pub fn new(store: Arc<dyn CredentialStore>) -> Self {
        Self {
            store,
            credentials: None,
            persisted: false,
        }
    }

    pub fn restore(&mut self) -> Result<AuthPersistence, AuthError> {
        self.credentials = self.store.load()?;
        self.persisted = self.credentials.is_some();
        Ok(self.persistence())
    }

    pub fn authorize(
        &mut self,
        authorization_code: SensitiveValue,
        oauth: &mut dyn DiscordOauth,
        now_unix_secs: u64,
    ) -> Result<AuthPersistence, AuthError> {
        let response = oauth.exchange(authorization_code.expose())?;
        self.install_response(response, now_unix_secs)
    }

    pub fn refresh_if_needed(
        &mut self,
        oauth: &mut dyn DiscordOauth,
        now_unix_secs: u64,
    ) -> Result<AuthPersistence, AuthError> {
        let credentials = self
            .credentials
            .as_ref()
            .ok_or(AuthError::AuthorizationExpired)?;
        if credentials.expires_at_unix_secs() > now_unix_secs.saturating_add(REFRESH_MARGIN_SECS) {
            return Ok(self.persistence());
        }
        self.refresh(oauth, now_unix_secs)
    }

    pub fn refresh_after_rejection(
        &mut self,
        oauth: &mut dyn DiscordOauth,
        now_unix_secs: u64,
    ) -> Result<AuthPersistence, AuthError> {
        match self.refresh(oauth, now_unix_secs) {
            Err(AuthError::Oauth(OauthError::Unauthorized)) => {
                self.invalidate()?;
                Err(AuthError::AuthorizationExpired)
            }
            result => result,
        }
    }

    pub fn sign_out(&mut self, oauth: &mut dyn DiscordOauth) -> Result<(), AuthError> {
        if let Some(credentials) = self.credentials.as_ref() {
            let _ = oauth.revoke(credentials.access_token());
        }
        self.store.delete()?;
        self.credentials = None;
        self.persisted = false;
        Ok(())
    }

    pub fn invalidate(&mut self) -> Result<(), AuthError> {
        self.store.delete()?;
        self.credentials = None;
        self.persisted = false;
        Ok(())
    }

    pub fn access_token(&self) -> Option<&str> {
        self.credentials
            .as_ref()
            .map(DiscordCredentials::access_token)
    }

    pub fn credentials_persisted(&self) -> bool {
        self.persisted
    }

    fn refresh(
        &mut self,
        oauth: &mut dyn DiscordOauth,
        now_unix_secs: u64,
    ) -> Result<AuthPersistence, AuthError> {
        let refresh_token = self
            .credentials
            .as_ref()
            .ok_or(AuthError::AuthorizationExpired)?
            .refresh_token();
        let response = oauth.refresh(refresh_token)?;
        self.install_response(response, now_unix_secs)
    }

    fn install_response(
        &mut self,
        mut response: TokenResponse,
        now_unix_secs: u64,
    ) -> Result<AuthPersistence, AuthError> {
        let expires_at = now_unix_secs
            .checked_add(response.expires_in_secs)
            .ok_or(AuthError::Oauth(OauthError::InvalidResponse))?;
        let credentials = DiscordCredentials::new(
            std::mem::take(&mut response.access_token),
            std::mem::take(&mut response.refresh_token),
            expires_at,
        )?;
        self.persisted = self.store.save(&credentials).is_ok();
        self.credentials = Some(credentials);
        Ok(self.persistence())
    }

    fn persistence(&self) -> AuthPersistence {
        if self.persisted {
            AuthPersistence::Persisted
        } else {
            AuthPersistence::MemoryOnly
        }
    }
}
