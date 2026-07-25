use std::{error::Error, fmt, io::Read, time::Duration};

use serde::{Deserialize, Serialize};
use url::Url;

const DEVICE_URL: &str = "https://id.twitch.tv/oauth2/device";
const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const VALIDATE_URL: &str = "https://id.twitch.tv/oauth2/validate";
const REVOKE_URL: &str = "https://id.twitch.tv/oauth2/revoke";
const USERS_URL: &str = "https://api.twitch.tv/helix/users";
const EVENTSUB_SUBSCRIPTIONS_URL: &str = "https://api.twitch.tv/helix/eventsub/subscriptions";
const SEND_CHAT_MESSAGE_URL: &str = "https://api.twitch.tv/helix/chat/messages";
const USER_AGENT: &str = concat!("PlayerVox-OverCrow/", env!("CARGO_PKG_VERSION"));
const OAUTH_BODY_MAX_BYTES: u64 = 64 * 1024;
const HELIX_BODY_MAX_BYTES: u64 = 256 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REQUIRED_SCOPES: &str = "user:read:chat user:write:chat";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpError {
    InvalidUrl,
    InvalidRequest,
    Transport,
    Timeout,
    Status(u16),
    BodyTooLarge,
    ProviderResponse,
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl => formatter.write_str("Twitch URL policy rejected the request"),
            Self::InvalidRequest => formatter.write_str("invalid Twitch request"),
            Self::Transport => formatter.write_str("Twitch transport failed"),
            Self::Timeout => formatter.write_str("Twitch request timed out"),
            Self::Status(status) => write!(formatter, "Twitch returned HTTP status {status}"),
            Self::BodyTooLarge => formatter.write_str("Twitch response exceeded its limit"),
            Self::ProviderResponse => formatter.write_str("Twitch returned an invalid response"),
        }
    }
}

impl Error for HttpError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceCodeGrant {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in_secs: u64,
    pub interval_secs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in_secs: u64,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenPoll {
    Pending,
    SlowDown,
    Denied,
    Expired,
    Authorized(TokenResponse),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenValidation {
    pub client_id: String,
    pub login: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub expires_in_secs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TwitchUser {
    pub id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatSubscription {
    Message,
    Clear,
    MessageDelete,
}

impl ChatSubscription {
    pub fn event_type(self) -> &'static str {
        match self {
            Self::Message => "channel.chat.message",
            Self::Clear => "channel.chat.clear",
            Self::MessageDelete => "channel.chat.message_delete",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatSendResult {
    pub message_id: Option<String>,
    pub is_sent: bool,
}

pub trait TwitchHttp: Send {
    fn begin_device_authorization(&mut self, client_id: &str)
    -> Result<DeviceCodeGrant, HttpError>;
    fn poll_device_token(
        &mut self,
        client_id: &str,
        device_code: &str,
    ) -> Result<TokenPoll, HttpError>;
    fn refresh_token(
        &mut self,
        client_id: &str,
        refresh_token: &str,
    ) -> Result<TokenResponse, HttpError>;
    fn validate_token(&mut self, access_token: &str) -> Result<TokenValidation, HttpError>;
    fn revoke_token(&mut self, client_id: &str, access_token: &str) -> Result<(), HttpError>;
    fn resolve_channel(
        &mut self,
        client_id: &str,
        access_token: &str,
        login: &str,
    ) -> Result<TwitchUser, HttpError>;
    fn create_chat_subscription(
        &mut self,
        client_id: &str,
        access_token: &str,
        session_id: &str,
        broadcaster_user_id: &str,
        user_id: &str,
        subscription: ChatSubscription,
    ) -> Result<(), HttpError>;
    fn send_chat_message(
        &mut self,
        client_id: &str,
        access_token: &str,
        broadcaster_user_id: &str,
        sender_id: &str,
        message: &str,
        reply_parent_message_id: Option<&str>,
    ) -> Result<ChatSendResult, HttpError>;
}

pub struct UreqTwitchHttp {
    agent: ureq::Agent,
}

impl Default for UreqTwitchHttp {
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

impl TwitchHttp for UreqTwitchHttp {
    fn begin_device_authorization(
        &mut self,
        client_id: &str,
    ) -> Result<DeviceCodeGrant, HttpError> {
        validate_client_id(client_id)?;
        let response = self
            .agent
            .post(DEVICE_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .send_form([("client_id", client_id), ("scopes", REQUIRED_SCOPES)])
            .map_err(map_transport)?;
        let body = read_success(response, OAUTH_BODY_MAX_BYTES)?;
        let wire: DeviceCodeWire =
            serde_json::from_slice(&body).map_err(|_| HttpError::ProviderResponse)?;
        let grant = DeviceCodeGrant {
            device_code: wire.device_code,
            user_code: wire.user_code,
            verification_uri: wire.verification_uri,
            expires_in_secs: wire.expires_in,
            interval_secs: wire.interval,
        };
        validate_grant(&grant)?;
        Ok(grant)
    }

    fn poll_device_token(
        &mut self,
        client_id: &str,
        device_code: &str,
    ) -> Result<TokenPoll, HttpError> {
        validate_client_id(client_id)?;
        validate_secret(device_code)?;
        let response = self
            .agent
            .post(TOKEN_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .send_form([
                ("client_id", client_id),
                ("scopes", REQUIRED_SCOPES),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .map_err(map_transport)?;
        let status = response.status().as_u16();
        let body = read_body(response, OAUTH_BODY_MAX_BYTES)?;
        if (200..300).contains(&status) {
            return parse_token_response(&body).map(TokenPoll::Authorized);
        }
        let error: OAuthErrorWire =
            serde_json::from_slice(&body).map_err(|_| HttpError::Status(status))?;
        match error.message.as_str() {
            "authorization_pending" => Ok(TokenPoll::Pending),
            "slow_down" => Ok(TokenPoll::SlowDown),
            "access_denied" => Ok(TokenPoll::Denied),
            "expired_token" => Ok(TokenPoll::Expired),
            _ => Err(HttpError::Status(status)),
        }
    }

    fn refresh_token(
        &mut self,
        client_id: &str,
        refresh_token: &str,
    ) -> Result<TokenResponse, HttpError> {
        validate_client_id(client_id)?;
        validate_secret(refresh_token)?;
        let response = self
            .agent
            .post(TOKEN_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .send_form([
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", client_id),
            ])
            .map_err(map_transport)?;
        let body = read_success(response, OAUTH_BODY_MAX_BYTES)?;
        parse_token_response(&body)
    }

    fn validate_token(&mut self, access_token: &str) -> Result<TokenValidation, HttpError> {
        validate_secret(access_token)?;
        let response = self
            .agent
            .get(VALIDATE_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .header("Authorization", format!("OAuth {access_token}"))
            .call()
            .map_err(map_transport)?;
        let body = read_success(response, OAUTH_BODY_MAX_BYTES)?;
        let wire: ValidationWire =
            serde_json::from_slice(&body).map_err(|_| HttpError::ProviderResponse)?;
        Ok(TokenValidation {
            client_id: wire.client_id,
            login: wire.login,
            user_id: wire.user_id,
            scopes: wire.scopes,
            expires_in_secs: wire.expires_in,
        })
    }

    fn revoke_token(&mut self, client_id: &str, access_token: &str) -> Result<(), HttpError> {
        validate_client_id(client_id)?;
        validate_secret(access_token)?;
        let response = self
            .agent
            .post(REVOKE_URL)
            .header("User-Agent", USER_AGENT)
            .send_form([("client_id", client_id), ("token", access_token)])
            .map_err(map_transport)?;
        read_success(response, OAUTH_BODY_MAX_BYTES).map(|_| ())
    }

    fn resolve_channel(
        &mut self,
        client_id: &str,
        access_token: &str,
        login: &str,
    ) -> Result<TwitchUser, HttpError> {
        validate_client_id(client_id)?;
        validate_secret(access_token)?;
        validate_login(login)?;
        let mut url = Url::parse(USERS_URL).map_err(|_| HttpError::InvalidUrl)?;
        url.query_pairs_mut().append_pair("login", login);
        let response = self
            .agent
            .get(url.as_str())
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .header("Client-Id", client_id)
            .header("Authorization", format!("Bearer {access_token}"))
            .call()
            .map_err(map_transport)?;
        let body = read_success(response, HELIX_BODY_MAX_BYTES)?;
        parse_user_response(&body, login)
    }

    fn create_chat_subscription(
        &mut self,
        client_id: &str,
        access_token: &str,
        session_id: &str,
        broadcaster_user_id: &str,
        user_id: &str,
        subscription: ChatSubscription,
    ) -> Result<(), HttpError> {
        validate_client_id(client_id)?;
        validate_secret(access_token)?;
        validate_provider_id(session_id)?;
        validate_numeric_id(broadcaster_user_id)?;
        validate_numeric_id(user_id)?;
        let request = SubscriptionRequest {
            event_type: subscription.event_type(),
            version: "1",
            condition: SubscriptionCondition {
                broadcaster_user_id,
                user_id,
            },
            transport: SubscriptionTransport {
                method: "websocket",
                session_id,
            },
        };
        let response = self
            .agent
            .post(EVENTSUB_SUBSCRIPTIONS_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .header("Client-Id", client_id)
            .header("Authorization", format!("Bearer {access_token}"))
            .send_json(request)
            .map_err(map_transport)?;
        let body = read_success(response, HELIX_BODY_MAX_BYTES)?;
        parse_subscription_response(&body, subscription)
    }

    fn send_chat_message(
        &mut self,
        client_id: &str,
        access_token: &str,
        broadcaster_user_id: &str,
        sender_id: &str,
        message: &str,
        reply_parent_message_id: Option<&str>,
    ) -> Result<ChatSendResult, HttpError> {
        validate_client_id(client_id)?;
        validate_secret(access_token)?;
        validate_numeric_id(broadcaster_user_id)?;
        validate_numeric_id(sender_id)?;
        validate_chat_text(message)?;
        if let Some(reply) = reply_parent_message_id {
            validate_provider_id(reply)?;
        }
        let request = SendChatRequest {
            broadcaster_id: broadcaster_user_id,
            sender_id,
            message,
            reply_parent_message_id,
        };
        let response = self
            .agent
            .post(SEND_CHAT_MESSAGE_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .header("Client-Id", client_id)
            .header("Authorization", format!("Bearer {access_token}"))
            .send_json(request)
            .map_err(map_transport)?;
        let body = read_success(response, HELIX_BODY_MAX_BYTES)?;
        parse_send_response(&body)
    }
}

pub fn validate_verification_uri(value: &str) -> Result<(), HttpError> {
    validate_exact_https(value, "www.twitch.tv")?;
    let parsed = Url::parse(value).map_err(|_| HttpError::InvalidUrl)?;
    if parsed.path() != "/activate" || parsed.fragment().is_some() {
        return Err(HttpError::InvalidUrl);
    }
    // Bare https://www.twitch.tv/activate is always accepted.
    if parsed.query().is_none() {
        return Ok(());
    }
    // Twitch Device Code returns either:
    //   /activate?device-code=<user_code>
    //   /activate?public=true&device-code=<user_code>
    // Reject any other query shape so Open Twitch cannot be steered off-site.
    let pairs = parsed.query_pairs().collect::<Vec<_>>();
    let device_codes: Vec<_> = pairs
        .iter()
        .filter(|(key, _)| key == "device-code")
        .collect();
    let device_code_ok = device_codes.len() == 1
        && device_codes[0].1.len() <= 128
        && !device_codes[0].1.is_empty()
        && device_codes[0]
            .1
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    let only_allowed_keys = pairs
        .iter()
        .all(|(key, _)| key == "device-code" || key == "public");
    let public_pairs: Vec<_> = pairs.iter().filter(|(key, _)| key == "public").collect();
    let public_ok =
        public_pairs.is_empty() || (public_pairs.len() == 1 && public_pairs[0].1 == "true");
    if matches!(pairs.len(), 1 | 2) && device_code_ok && only_allowed_keys && public_ok {
        return Ok(());
    }
    Err(HttpError::InvalidUrl)
}

fn validate_exact_https(value: &str, host: &str) -> Result<(), HttpError> {
    let parsed = Url::parse(value).map_err(|_| HttpError::InvalidUrl)?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !parsed
            .host_str()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(host))
        || parsed.port_or_known_default() != Some(443)
        || parsed.fragment().is_some()
    {
        return Err(HttpError::InvalidUrl);
    }
    Ok(())
}

fn validate_grant(grant: &DeviceCodeGrant) -> Result<(), HttpError> {
    validate_verification_uri(&grant.verification_uri)?;
    validate_secret(&grant.device_code)?;
    if grant.user_code.is_empty()
        || grant.user_code.len() > 64
        || !grant
            .user_code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !(1..=60).contains(&grant.interval_secs)
        || !(1..=1_800).contains(&grant.expires_in_secs)
    {
        return Err(HttpError::ProviderResponse);
    }
    Ok(())
}

fn validate_client_id(value: &str) -> Result<(), HttpError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(HttpError::InvalidRequest);
    }
    Ok(())
}

fn validate_secret(value: &str) -> Result<(), HttpError> {
    if value.is_empty()
        || value.len() > 4 * 1024
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(HttpError::InvalidRequest);
    }
    Ok(())
}

fn validate_login(value: &str) -> Result<(), HttpError> {
    if value.is_empty()
        || value.len() > 25
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(HttpError::InvalidRequest);
    }
    Ok(())
}

fn validate_provider_id(value: &str) -> Result<(), HttpError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(HttpError::InvalidRequest);
    }
    Ok(())
}

fn validate_numeric_id(value: &str) -> Result<(), HttpError> {
    if value.is_empty() || value.len() > 32 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HttpError::InvalidRequest);
    }
    Ok(())
}

fn validate_chat_text(value: &str) -> Result<(), HttpError> {
    if value.is_empty()
        || value.chars().take(501).count() > 500
        || value.chars().any(char::is_control)
    {
        return Err(HttpError::InvalidRequest);
    }
    Ok(())
}

fn read_success(
    response: ureq::http::Response<ureq::Body>,
    max: u64,
) -> Result<Vec<u8>, HttpError> {
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(HttpError::Status(status));
    }
    read_body(response, max)
}

fn read_body(
    mut response: ureq::http::Response<ureq::Body>,
    max: u64,
) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(max.saturating_add(1))
        .read_to_end(&mut body)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                HttpError::Timeout
            } else {
                HttpError::Transport
            }
        })?;
    if body.len() as u64 > max {
        return Err(HttpError::BodyTooLarge);
    }
    Ok(body)
}

fn parse_token_response(body: &[u8]) -> Result<TokenResponse, HttpError> {
    let wire: TokenWire = serde_json::from_slice(body).map_err(|_| HttpError::ProviderResponse)?;
    validate_secret(&wire.access_token)?;
    validate_secret(&wire.refresh_token)?;
    if wire.expires_in == 0
        || wire.expires_in > 31_536_000
        || wire.scope.len() > 16
        || wire.scope.iter().any(|scope| {
            scope.is_empty()
                || scope.len() > 128
                || scope.bytes().any(|byte| byte.is_ascii_control())
        })
    {
        return Err(HttpError::ProviderResponse);
    }
    Ok(TokenResponse {
        access_token: wire.access_token,
        refresh_token: wire.refresh_token,
        expires_in_secs: wire.expires_in,
        scopes: wire.scope,
    })
}

fn map_transport(error: ureq::Error) -> HttpError {
    match error {
        ureq::Error::Timeout(_) => HttpError::Timeout,
        _ => HttpError::Transport,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceCodeWire {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct OAuthErrorWire {
    message: String,
}

#[derive(Deserialize)]
struct TokenWire {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    scope: Vec<String>,
}

#[derive(Deserialize)]
struct ValidationWire {
    client_id: String,
    login: String,
    scopes: Vec<String>,
    user_id: String,
    expires_in: u64,
}

#[derive(Serialize)]
struct SubscriptionRequest<'a> {
    #[serde(rename = "type")]
    event_type: &'a str,
    version: &'static str,
    condition: SubscriptionCondition<'a>,
    transport: SubscriptionTransport<'a>,
}

#[derive(Serialize)]
struct SubscriptionCondition<'a> {
    broadcaster_user_id: &'a str,
    user_id: &'a str,
}

#[derive(Serialize)]
struct SubscriptionTransport<'a> {
    method: &'static str,
    session_id: &'a str,
}

#[derive(Serialize)]
struct SendChatRequest<'a> {
    broadcaster_id: &'a str,
    sender_id: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_parent_message_id: Option<&'a str>,
}

#[derive(Deserialize)]
struct DataResponse<T> {
    data: Vec<T>,
}

#[derive(Deserialize)]
struct UserWire {
    id: String,
    login: String,
}

#[derive(Deserialize)]
struct SubscriptionWire {
    id: String,
    status: String,
    #[serde(rename = "type")]
    event_type: String,
}

#[derive(Deserialize)]
struct SendWire {
    #[serde(default)]
    message_id: String,
    is_sent: bool,
}

pub(super) fn parse_user_response(
    body: &[u8],
    expected_login: &str,
) -> Result<TwitchUser, HttpError> {
    validate_login(expected_login).map_err(|_| HttpError::ProviderResponse)?;
    let response: DataResponse<UserWire> =
        serde_json::from_slice(body).map_err(|_| HttpError::ProviderResponse)?;
    let [wire] = response.data.as_slice() else {
        return Err(HttpError::ProviderResponse);
    };
    validate_numeric_id(&wire.id).map_err(|_| HttpError::ProviderResponse)?;
    validate_login(&wire.login).map_err(|_| HttpError::ProviderResponse)?;
    if wire.login != expected_login {
        return Err(HttpError::ProviderResponse);
    }
    Ok(TwitchUser {
        id: wire.id.clone(),
    })
}

pub(super) fn parse_subscription_response(
    body: &[u8],
    expected: ChatSubscription,
) -> Result<(), HttpError> {
    let response: DataResponse<SubscriptionWire> =
        serde_json::from_slice(body).map_err(|_| HttpError::ProviderResponse)?;
    let [wire] = response.data.as_slice() else {
        return Err(HttpError::ProviderResponse);
    };
    validate_provider_id(&wire.id).map_err(|_| HttpError::ProviderResponse)?;
    if wire.status != "enabled" || wire.event_type != expected.event_type() {
        return Err(HttpError::ProviderResponse);
    }
    Ok(())
}

pub(super) fn parse_send_response(body: &[u8]) -> Result<ChatSendResult, HttpError> {
    let response: DataResponse<SendWire> =
        serde_json::from_slice(body).map_err(|_| HttpError::ProviderResponse)?;
    let [wire] = response.data.as_slice() else {
        return Err(HttpError::ProviderResponse);
    };
    let message_id = if wire.message_id.is_empty() {
        None
    } else {
        validate_provider_id(&wire.message_id).map_err(|_| HttpError::ProviderResponse)?;
        Some(wire.message_id.clone())
    };
    if wire.is_sent && message_id.is_none() {
        return Err(HttpError::ProviderResponse);
    }
    Ok(ChatSendResult {
        message_id,
        is_sent: wire.is_sent,
    })
}
