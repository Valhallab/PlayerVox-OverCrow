use std::{error::Error, fmt, sync::Arc};

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

const CREDENTIAL_SCHEMA_VERSION: u32 = 1;
const TOKEN_MAX_BYTES: usize = 4 * 1024;
const CREDENTIAL_MAX_BYTES: usize = 12 * 1024;
const KEYRING_SERVICE: &str = "com.playervox.OverCrow";
const KEYRING_USER: &str = "twitch-oauth-v1";

pub struct TwitchCredentials {
    access_token: Zeroizing<String>,
    refresh_token: Zeroizing<String>,
    expires_at_unix_secs: u64,
}

impl TwitchCredentials {
    pub fn new(
        access_token: impl Into<String>,
        refresh_token: impl Into<String>,
        expires_at_unix_secs: u64,
    ) -> Result<Self, CredentialStoreError> {
        let access_token = access_token.into();
        let refresh_token = refresh_token.into();
        if !valid_token(&access_token) || !valid_token(&refresh_token) || expires_at_unix_secs == 0
        {
            return Err(CredentialStoreError::Corrupt);
        }
        Ok(Self {
            access_token: Zeroizing::new(access_token),
            refresh_token: Zeroizing::new(refresh_token),
            expires_at_unix_secs,
        })
    }

    pub fn access_token(&self) -> &str {
        self.access_token.as_str()
    }

    pub fn refresh_token(&self) -> &str {
        self.refresh_token.as_str()
    }

    pub fn expires_at_unix_secs(&self) -> u64 {
        self.expires_at_unix_secs
    }

    pub(crate) fn set_expiry(&mut self, expires_at_unix_secs: u64) {
        self.expires_at_unix_secs = expires_at_unix_secs;
    }

    pub fn duplicate(&self) -> Self {
        Self {
            access_token: Zeroizing::new(self.access_token.to_string()),
            refresh_token: Zeroizing::new(self.refresh_token.to_string()),
            expires_at_unix_secs: self.expires_at_unix_secs,
        }
    }
}

impl fmt::Debug for TwitchCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TwitchCredentials")
            .field("secrets", &"[REDACTED]")
            .field("expires_at_unix_secs", &self.expires_at_unix_secs)
            .finish()
    }
}

impl Drop for TwitchCredentials {
    fn drop(&mut self) {
        self.expires_at_unix_secs.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialStoreError {
    Unavailable,
    Corrupt,
}

impl fmt::Display for CredentialStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("secure credential storage is unavailable"),
            Self::Corrupt => formatter.write_str("stored credential is invalid"),
        }
    }
}

impl Error for CredentialStoreError {}

pub trait CredentialStore: Send + Sync {
    fn load(&self) -> Result<Option<TwitchCredentials>, CredentialStoreError>;
    fn save(&self, credentials: &TwitchCredentials) -> Result<(), CredentialStoreError>;
    fn delete(&self) -> Result<(), CredentialStoreError>;
}

pub struct SecretServiceCredentialStore;

impl CredentialStore for SecretServiceCredentialStore {
    fn load(&self) -> Result<Option<TwitchCredentials>, CredentialStoreError> {
        let entry = keyring_entry()?;
        let secret = match entry.get_secret() {
            Ok(secret) => Zeroizing::new(secret),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(_) => return Err(CredentialStoreError::Unavailable),
        };
        decode_credentials(&secret).map(Some)
    }

    fn save(&self, credentials: &TwitchCredentials) -> Result<(), CredentialStoreError> {
        let entry = keyring_entry()?;
        let encoded = encode_credentials(credentials)?;
        entry
            .set_secret(&encoded)
            .map_err(|_| CredentialStoreError::Unavailable)
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        let entry = keyring_entry()?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(CredentialStoreError::Unavailable),
        }
    }
}

pub fn default_credential_store() -> Arc<dyn CredentialStore> {
    Arc::new(SecretServiceCredentialStore)
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct CredentialWireRef<'a> {
    schema_version: u32,
    access_token: &'a str,
    refresh_token: &'a str,
    expires_at_unix_secs: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialWire {
    schema_version: u32,
    access_token: String,
    refresh_token: String,
    expires_at_unix_secs: u64,
}

pub fn encode_credentials(
    credentials: &TwitchCredentials,
) -> Result<Zeroizing<Vec<u8>>, CredentialStoreError> {
    let wire = CredentialWireRef {
        schema_version: CREDENTIAL_SCHEMA_VERSION,
        access_token: credentials.access_token(),
        refresh_token: credentials.refresh_token(),
        expires_at_unix_secs: credentials.expires_at_unix_secs(),
    };
    let encoded = serde_json::to_vec(&wire).map_err(|_| CredentialStoreError::Corrupt)?;
    if encoded.len() > CREDENTIAL_MAX_BYTES {
        return Err(CredentialStoreError::Corrupt);
    }
    Ok(Zeroizing::new(encoded))
}

pub fn decode_credentials(bytes: &[u8]) -> Result<TwitchCredentials, CredentialStoreError> {
    if bytes.len() > CREDENTIAL_MAX_BYTES {
        return Err(CredentialStoreError::Corrupt);
    }
    let mut wire: CredentialWire =
        serde_json::from_slice(bytes).map_err(|_| CredentialStoreError::Corrupt)?;
    if wire.schema_version != CREDENTIAL_SCHEMA_VERSION {
        wire.access_token.zeroize();
        wire.refresh_token.zeroize();
        return Err(CredentialStoreError::Corrupt);
    }
    TwitchCredentials::new(
        std::mem::take(&mut wire.access_token),
        std::mem::take(&mut wire.refresh_token),
        wire.expires_at_unix_secs,
    )
}

fn keyring_entry() -> Result<keyring::Entry, CredentialStoreError> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|_| CredentialStoreError::Unavailable)
}

fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= TOKEN_MAX_BYTES
        && !token
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
}
