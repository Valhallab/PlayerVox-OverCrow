use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use super::model::ParsedChatMessage;
use super::model::TwitchReplyContext;

pub const EVENTSUB_MESSAGE_MAX_BYTES: usize = 64 * 1024;
const EVENTSUB_ID_MAX_BYTES: usize = 256;
const EVENTSUB_KEEPALIVE_MAX_SECS: u64 = 10 * 60;
const CHAT_TEXT_MAX_CHARS: usize = 500;
const DISPLAY_NAME_MAX_CHARS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventSubMessage {
    pub delivery_id: String,
    pub kind: EventSubKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventSubKind {
    Welcome {
        session_id: String,
        keepalive_timeout: Duration,
    },
    Keepalive,
    Reconnect {
        url: String,
    },
    Revocation(EventSubRevocation),
    ChatMessage {
        broadcaster_user_id: String,
        message: ParsedChatMessage,
    },
    ChatClear {
        broadcaster_user_id: String,
    },
    ChatMessageDelete {
        broadcaster_user_id: String,
        message_id: String,
    },
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventSubRevocation {
    Authentication,
    ChannelUnavailable,
    Provider,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventSubParseError {
    Oversized,
    Malformed,
}

pub fn parse_eventsub_message(raw: &str) -> Result<EventSubMessage, EventSubParseError> {
    if raw.len() > EVENTSUB_MESSAGE_MAX_BYTES {
        return Err(EventSubParseError::Oversized);
    }
    let envelope: Envelope =
        serde_json::from_str(raw).map_err(|_| EventSubParseError::Malformed)?;
    if !valid_provider_id(&envelope.metadata.message_id) {
        return Err(EventSubParseError::Malformed);
    }

    let kind = match envelope.metadata.message_type.as_str() {
        "session_welcome" => {
            let session = parse_session(envelope.payload)?;
            let keepalive = session
                .keepalive_timeout_seconds
                .filter(|seconds| (1..=EVENTSUB_KEEPALIVE_MAX_SECS).contains(seconds))
                .ok_or(EventSubParseError::Malformed)?;
            EventSubKind::Welcome {
                session_id: session.id,
                keepalive_timeout: Duration::from_secs(keepalive),
            }
        }
        "session_keepalive" => EventSubKind::Keepalive,
        "session_reconnect" => {
            let session = parse_session(envelope.payload)?;
            let url = session
                .reconnect_url
                .filter(|url| valid_eventsub_url(url))
                .ok_or(EventSubParseError::Malformed)?;
            EventSubKind::Reconnect { url }
        }
        "revocation" => EventSubKind::Revocation(parse_revocation(envelope.payload)?),
        "notification" => parse_notification(
            envelope.metadata.subscription_type.as_deref(),
            envelope.metadata.subscription_version.as_deref(),
            envelope.payload,
        )?,
        _ => EventSubKind::Unsupported,
    };

    Ok(EventSubMessage {
        delivery_id: envelope.metadata.message_id,
        kind,
    })
}

#[derive(Deserialize)]
struct Envelope {
    metadata: Metadata,
    #[serde(default)]
    payload: Value,
}

#[derive(Deserialize)]
struct Metadata {
    message_id: String,
    message_type: String,
    #[serde(default)]
    subscription_type: Option<String>,
    #[serde(default)]
    subscription_version: Option<String>,
}

#[derive(Deserialize)]
struct SessionPayload {
    session: SessionWire,
}

#[derive(Deserialize)]
struct SessionWire {
    id: String,
    #[serde(default)]
    keepalive_timeout_seconds: Option<u64>,
    #[serde(default)]
    reconnect_url: Option<String>,
}

#[derive(Deserialize)]
struct NotificationPayload<T> {
    event: T,
}

#[derive(Deserialize)]
struct ChatMessageWire {
    broadcaster_user_id: String,
    chatter_user_name: String,
    message_id: String,
    #[serde(default)]
    color: Option<String>,
    message: ChatTextWire,
    #[serde(default)]
    reply: Option<ReplyWire>,
}

#[derive(Deserialize)]
struct ChatTextWire {
    text: String,
}

#[derive(Deserialize)]
struct ReplyWire {
    parent_message_id: String,
    parent_message_body: String,
    parent_user_name: String,
}

#[derive(Deserialize)]
struct ClearWire {
    broadcaster_user_id: String,
}

#[derive(Deserialize)]
struct DeleteWire {
    broadcaster_user_id: String,
    message_id: String,
}

#[derive(Deserialize)]
struct RevocationPayload {
    subscription: RevocationSubscription,
}

#[derive(Deserialize)]
struct RevocationSubscription {
    status: String,
}

fn parse_session(payload: Value) -> Result<SessionWire, EventSubParseError> {
    let payload: SessionPayload =
        serde_json::from_value(payload).map_err(|_| EventSubParseError::Malformed)?;
    if !valid_provider_id(&payload.session.id) {
        return Err(EventSubParseError::Malformed);
    }
    Ok(payload.session)
}

fn parse_notification(
    subscription_type: Option<&str>,
    subscription_version: Option<&str>,
    payload: Value,
) -> Result<EventSubKind, EventSubParseError> {
    if subscription_version != Some("1") {
        return Err(EventSubParseError::Malformed);
    }
    match subscription_type {
        Some("channel.chat.message") => {
            let payload: NotificationPayload<ChatMessageWire> =
                serde_json::from_value(payload).map_err(|_| EventSubParseError::Malformed)?;
            let broadcaster_user_id = payload.event.broadcaster_user_id.clone();
            parse_chat_message(payload.event).map(|message| EventSubKind::ChatMessage {
                broadcaster_user_id,
                message,
            })
        }
        Some("channel.chat.clear") => {
            let payload: NotificationPayload<ClearWire> =
                serde_json::from_value(payload).map_err(|_| EventSubParseError::Malformed)?;
            if !valid_numeric_id(&payload.event.broadcaster_user_id) {
                return Err(EventSubParseError::Malformed);
            }
            Ok(EventSubKind::ChatClear {
                broadcaster_user_id: payload.event.broadcaster_user_id,
            })
        }
        Some("channel.chat.message_delete") => {
            let payload: NotificationPayload<DeleteWire> =
                serde_json::from_value(payload).map_err(|_| EventSubParseError::Malformed)?;
            if !valid_numeric_id(&payload.event.broadcaster_user_id)
                || !valid_provider_id(&payload.event.message_id)
            {
                return Err(EventSubParseError::Malformed);
            }
            Ok(EventSubKind::ChatMessageDelete {
                broadcaster_user_id: payload.event.broadcaster_user_id,
                message_id: payload.event.message_id,
            })
        }
        Some(_) => Ok(EventSubKind::Unsupported),
        None => Err(EventSubParseError::Malformed),
    }
}

fn parse_revocation(payload: Value) -> Result<EventSubRevocation, EventSubParseError> {
    let payload: RevocationPayload =
        serde_json::from_value(payload).map_err(|_| EventSubParseError::Malformed)?;
    match payload.subscription.status.as_str() {
        "authorization_revoked" | "user_removed" => Ok(EventSubRevocation::Authentication),
        "chat_user_banned" => Ok(EventSubRevocation::ChannelUnavailable),
        "version_removed" | "notification_failures_exceeded" => Ok(EventSubRevocation::Provider),
        _ => Err(EventSubParseError::Malformed),
    }
}

fn parse_chat_message(wire: ChatMessageWire) -> Result<ParsedChatMessage, EventSubParseError> {
    if !valid_numeric_id(&wire.broadcaster_user_id)
        || !valid_text(&wire.chatter_user_name, DISPLAY_NAME_MAX_CHARS)
        || !valid_provider_id(&wire.message_id)
        || !valid_text(&wire.message.text, CHAT_TEXT_MAX_CHARS)
    {
        return Err(EventSubParseError::Malformed);
    }
    let name_color = wire.color.as_deref().and_then(parse_color);
    let reply = wire.reply.map(parse_reply).transpose()?;
    Ok(ParsedChatMessage {
        id: wire.message_id,
        display_name: wire.chatter_user_name,
        name_color,
        text: wire.message.text,
        reply,
    })
}

fn parse_reply(wire: ReplyWire) -> Result<TwitchReplyContext, EventSubParseError> {
    if !valid_provider_id(&wire.parent_message_id)
        || !valid_text(&wire.parent_user_name, DISPLAY_NAME_MAX_CHARS)
        || !valid_text(&wire.parent_message_body, CHAT_TEXT_MAX_CHARS)
    {
        return Err(EventSubParseError::Malformed);
    }
    Ok(TwitchReplyContext {
        message_id: wire.parent_message_id,
        display_name: wire.parent_user_name,
        body: wire.parent_message_body,
    })
}

fn parse_color(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([red, green, blue])
}

fn valid_provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= EVENTSUB_ID_MAX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_numeric_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_text(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().take(max_chars.saturating_add(1)).count() <= max_chars
        && !value.chars().any(char::is_control)
}

pub(super) fn valid_eventsub_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    url.scheme() == "wss"
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("eventsub.wss.twitch.tv"))
        && url.port_or_known_default() == Some(443)
        && url.path() == "/ws"
        && url.fragment().is_none()
}
