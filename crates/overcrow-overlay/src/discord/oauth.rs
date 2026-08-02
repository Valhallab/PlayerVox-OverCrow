use std::{error::Error, fmt, io::Read, time::Duration};

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

const TOKEN_URL: &str = "https://api.playervox.com/api/v1/overcrow/discord/oauth/token";
const REFRESH_URL: &str = "https://api.playervox.com/api/v1/overcrow/discord/oauth/refresh";
const REVOKE_URL: &str = "https://api.playervox.com/api/v1/overcrow/discord/oauth/revoke";
const USER_AGENT: &str = concat!("PlayerVox-OverCrow/", env!("CARGO_PKG_VERSION"));
const RESPONSE_MAX_BYTES: u64 = 16 * 1024;
const TOKEN_MAX_BYTES: usize = 4 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OauthError {
    InvalidRequest,
    Transport,
    Timeout,
    Status(u16),
    BodyTooLarge,
    InvalidResponse,
    Unauthorized,
}

impl fmt::Display for OauthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest => formatter.write_str("invalid Discord OAuth request"),
            Self::Transport => formatter.write_str("Discord OAuth transport failed"),
            Self::Timeout => formatter.write_str("Discord OAuth request timed out"),
            Self::Status(status) => {
                write!(formatter, "Discord OAuth returned HTTP status {status}")
            }
            Self::BodyTooLarge => formatter.write_str("Discord OAuth response exceeded its limit"),
            Self::InvalidResponse => {
                formatter.write_str("Discord OAuth returned an invalid response")
            }
            Self::Unauthorized => formatter.write_str("Discord authorization expired"),
        }
    }
}

impl Error for OauthError {}

#[derive(Eq, PartialEq)]
pub struct TokenResponse {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_in_secs: u64,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenResponse")
            .field("secrets", &"[REDACTED]")
            .field("expires_in_secs", &self.expires_in_secs)
            .finish()
    }
}

impl Drop for TokenResponse {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.expires_in_secs.zeroize();
    }
}

pub trait DiscordOauth: Send {
    fn exchange(&mut self, authorization_code: &str) -> Result<TokenResponse, OauthError>;
    fn refresh(&mut self, refresh_token: &str) -> Result<TokenResponse, OauthError>;
    fn revoke(&mut self, access_token: &str) -> Result<(), OauthError>;
}

pub struct UreqDiscordOauth {
    agent: ureq::Agent,
}

impl Default for UreqDiscordOauth {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(REQUEST_TIMEOUT))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into(),
        }
    }
}

impl DiscordOauth for UreqDiscordOauth {
    fn exchange(&mut self, authorization_code: &str) -> Result<TokenResponse, OauthError> {
        validate_secret(authorization_code)?;
        self.post_token(TOKEN_URL, &TokenRequest { authorization_code })
    }

    fn refresh(&mut self, refresh_token: &str) -> Result<TokenResponse, OauthError> {
        validate_secret(refresh_token)?;
        self.post_token(REFRESH_URL, &RefreshRequest { refresh_token })
    }

    fn revoke(&mut self, access_token: &str) -> Result<(), OauthError> {
        validate_secret(access_token)?;
        let response = self
            .agent
            .post(REVOKE_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .send_json(&RevokeRequest { access_token })
            .map_err(map_transport)?;
        let status = response.status().as_u16();
        if matches!(status, 401 | 403) {
            return Err(OauthError::Unauthorized);
        }
        if !(200..300).contains(&status) {
            return Err(OauthError::Status(status));
        }
        Ok(())
    }
}

impl UreqDiscordOauth {
    fn post_token<T: Serialize>(
        &self,
        url: &str,
        request: &T,
    ) -> Result<TokenResponse, OauthError> {
        let response = self
            .agent
            .post(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .send_json(request)
            .map_err(map_transport)?;
        let status = response.status().as_u16();
        if matches!(status, 401 | 403) {
            return Err(OauthError::Unauthorized);
        }
        if !(200..300).contains(&status) {
            return Err(OauthError::Status(status));
        }
        let body = read_body(response)?;
        parse_token_response(&body)
    }
}

#[derive(Serialize)]
struct TokenRequest<'a> {
    authorization_code: &'a str,
}

#[derive(Serialize)]
struct RefreshRequest<'a> {
    refresh_token: &'a str,
}

#[derive(Serialize)]
struct RevokeRequest<'a> {
    access_token: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenWire {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

impl Drop for TokenWire {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.expires_in.zeroize();
    }
}

pub(crate) fn parse_token_response(body: &[u8]) -> Result<TokenResponse, OauthError> {
    let mut wire: TokenWire =
        serde_json::from_slice(body).map_err(|_| OauthError::InvalidResponse)?;
    validate_secret(&wire.access_token).map_err(|_| OauthError::InvalidResponse)?;
    validate_secret(&wire.refresh_token).map_err(|_| OauthError::InvalidResponse)?;
    if wire.expires_in == 0 || wire.expires_in > 31_536_000 {
        return Err(OauthError::InvalidResponse);
    }
    Ok(TokenResponse {
        access_token: std::mem::take(&mut wire.access_token),
        refresh_token: std::mem::take(&mut wire.refresh_token),
        expires_in_secs: wire.expires_in,
    })
}

fn validate_secret(value: &str) -> Result<(), OauthError> {
    if value.is_empty()
        || value.len() > TOKEN_MAX_BYTES
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(OauthError::InvalidRequest);
    }
    Ok(())
}

fn read_body(mut response: ureq::http::Response<ureq::Body>) -> Result<Vec<u8>, OauthError> {
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(RESPONSE_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut body)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                OauthError::Timeout
            } else {
                OauthError::Transport
            }
        })?;
    if body.len() as u64 > RESPONSE_MAX_BYTES {
        return Err(OauthError::BodyTooLarge);
    }
    Ok(body)
}

fn map_transport(error: ureq::Error) -> OauthError {
    match error {
        ureq::Error::Timeout(_) => OauthError::Timeout,
        _ => OauthError::Transport,
    }
}
