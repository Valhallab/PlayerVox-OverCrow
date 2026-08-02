use serde::Deserialize;
use serde_json::Value;

use super::model::{
    DiscordRpcEvent, SensitiveValue, VoiceChannel, VoiceParticipant, VoiceSubscriptionEvent,
};

pub const DISCORD_RPC_MESSAGE_MAX_BYTES: usize = 256 * 1024;
const DISCORD_PARTICIPANT_MAX: usize = 64;
const DISCORD_ID_MAX_BYTES: usize = 32;
const DISCORD_NAME_MAX_CHARS: usize = 128;
const DISCORD_AVATAR_HASH_MAX_BYTES: usize = 128;
const DISCORD_CODE_MAX_BYTES: usize = 1024;
const DISCORD_COMMAND_MAX_BYTES: usize = 64;
const DISCORD_EVENT_MAX_BYTES: usize = 64;
const DISCORD_NONCE_MAX_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpcParseError {
    Oversized,
    Malformed,
    InvalidData,
}

#[derive(Deserialize)]
struct WireEnvelope {
    cmd: String,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    evt: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
}

#[derive(Deserialize)]
struct WireAuthorization {
    code: String,
}

#[derive(Deserialize)]
struct WireAuthentication {
    user: WireUser,
}

#[derive(Deserialize)]
struct WireChannelSelect {
    #[serde(default)]
    channel_id: Option<String>,
}

#[derive(Deserialize)]
struct WireChannel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    voice_states: Vec<WireVoiceState>,
}

#[derive(Deserialize)]
struct WireVoiceState {
    #[serde(default)]
    nick: Option<String>,
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    speaking: bool,
    #[serde(default)]
    voice_state: WireVoiceFlags,
    user: WireUser,
}

#[derive(Default, Deserialize)]
struct WireVoiceFlags {
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    deaf: bool,
    #[serde(default)]
    self_mute: bool,
    #[serde(default)]
    self_deaf: bool,
    #[serde(default)]
    suppress: bool,
}

#[derive(Deserialize)]
struct WireUser {
    id: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    global_name: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
}

#[derive(Deserialize)]
struct WireSpeaking {
    user_id: String,
}

#[derive(Deserialize)]
struct WireDeletedUser {
    user: WireUserId,
}

#[derive(Deserialize)]
struct WireUserId {
    id: String,
}

#[derive(Deserialize)]
struct WireProviderError {
    code: i64,
}

#[derive(Deserialize)]
struct WireSubscription {
    evt: String,
}

pub fn parse_rpc_message(bytes: &[u8]) -> Result<DiscordRpcEvent, RpcParseError> {
    if bytes.len() > DISCORD_RPC_MESSAGE_MAX_BYTES {
        return Err(RpcParseError::Oversized);
    }
    let envelope: WireEnvelope =
        serde_json::from_slice(bytes).map_err(|_| RpcParseError::Malformed)?;
    if !valid_bounded_ascii(&envelope.cmd, DISCORD_COMMAND_MAX_BYTES) {
        return Err(RpcParseError::InvalidData);
    }
    if envelope
        .evt
        .as_deref()
        .is_some_and(|event| !valid_bounded_ascii(event, DISCORD_EVENT_MAX_BYTES))
    {
        return Err(RpcParseError::InvalidData);
    }
    if envelope
        .nonce
        .as_deref()
        .is_some_and(|nonce| !valid_bounded_ascii(nonce, DISCORD_NONCE_MAX_BYTES))
    {
        return Err(RpcParseError::InvalidData);
    }
    parse_envelope(envelope)
}

fn parse_envelope(envelope: WireEnvelope) -> Result<DiscordRpcEvent, RpcParseError> {
    if envelope.evt.as_deref() == Some("ERROR") {
        let error: WireProviderError = parse_data(envelope.data)?;
        return Ok(DiscordRpcEvent::ProviderError {
            command: Some(envelope.cmd),
            code: error.code,
            nonce: envelope.nonce,
        });
    }

    match (envelope.cmd.as_str(), envelope.evt.as_deref()) {
        ("DISPATCH", Some("READY")) => Ok(DiscordRpcEvent::Ready),
        ("AUTHORIZE", _) => parse_authorization(envelope.data, envelope.nonce),
        ("AUTHENTICATE", _) => parse_authentication(envelope.data, envelope.nonce),
        ("GET_SELECTED_VOICE_CHANNEL", _) => parse_channel(envelope.data, envelope.nonce),
        ("DISPATCH", Some("VOICE_CHANNEL_SELECT")) => parse_channel_select(envelope.data),
        ("DISPATCH", Some("VOICE_STATE_CREATE")) => {
            parse_voice_state(envelope.data).map(DiscordRpcEvent::ParticipantCreated)
        }
        ("DISPATCH", Some("VOICE_STATE_UPDATE")) => {
            parse_voice_state(envelope.data).map(DiscordRpcEvent::ParticipantUpdated)
        }
        ("DISPATCH", Some("VOICE_STATE_DELETE")) => parse_deleted(envelope.data),
        ("DISPATCH", Some("SPEAKING_START")) => parse_speaking(envelope.data, true),
        ("DISPATCH", Some("SPEAKING_STOP")) => parse_speaking(envelope.data, false),
        ("SUBSCRIBE", _) => parse_subscription(envelope.data, envelope.nonce, true),
        ("UNSUBSCRIBE", _) => parse_subscription(envelope.data, envelope.nonce, false),
        _ => Ok(DiscordRpcEvent::Ignored),
    }
}

fn parse_authorization(
    data: Value,
    nonce: Option<String>,
) -> Result<DiscordRpcEvent, RpcParseError> {
    let authorization: WireAuthorization = parse_data(data)?;
    if !valid_sensitive(&authorization.code, DISCORD_CODE_MAX_BYTES) {
        return Err(RpcParseError::InvalidData);
    }
    Ok(DiscordRpcEvent::AuthorizationGranted {
        nonce: nonce.ok_or(RpcParseError::InvalidData)?,
        code: SensitiveValue::new(authorization.code),
    })
}

fn parse_authentication(
    data: Value,
    nonce: Option<String>,
) -> Result<DiscordRpcEvent, RpcParseError> {
    let authentication: WireAuthentication = parse_data(data)?;
    let user_id = valid_id(authentication.user.id)?;
    Ok(DiscordRpcEvent::Authenticated {
        nonce: nonce.ok_or(RpcParseError::InvalidData)?,
        user_id,
    })
}

fn parse_channel_select(data: Value) -> Result<DiscordRpcEvent, RpcParseError> {
    let selection: WireChannelSelect = parse_data(data)?;
    let channel_id = selection.channel_id.map(valid_id).transpose()?;
    Ok(DiscordRpcEvent::ChannelSelected(channel_id))
}

fn parse_channel(data: Value, nonce: Option<String>) -> Result<DiscordRpcEvent, RpcParseError> {
    let nonce = nonce.ok_or(RpcParseError::InvalidData)?;
    if data.is_null() {
        return Ok(DiscordRpcEvent::ChannelSnapshot {
            nonce,
            channel: None,
        });
    }
    let channel: WireChannel = parse_data(data)?;
    if channel.voice_states.len() > DISCORD_PARTICIPANT_MAX {
        return Err(RpcParseError::InvalidData);
    }
    let id = valid_id(channel.id)?;
    let name = channel
        .name
        .filter(|name| !name.is_empty())
        .map(valid_name)
        .transpose()?
        .unwrap_or_else(|| "Direct call".to_owned());
    let participants = channel
        .voice_states
        .into_iter()
        .map(validate_voice_state)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DiscordRpcEvent::ChannelSnapshot {
        nonce,
        channel: Some(VoiceChannel {
            id,
            name,
            participants,
        }),
    })
}

fn parse_voice_state(data: Value) -> Result<VoiceParticipant, RpcParseError> {
    validate_voice_state(parse_data(data)?)
}

fn validate_voice_state(state: WireVoiceState) -> Result<VoiceParticipant, RpcParseError> {
    let id = valid_id(state.user.id)?;
    let display_name = [
        state.nick,
        state.user.global_name,
        Some(state.user.username),
    ]
    .into_iter()
    .flatten()
    .find(|candidate| !candidate.is_empty())
    .ok_or(RpcParseError::InvalidData)
    .and_then(valid_name)?;
    let avatar_hash = state.user.avatar.map(valid_avatar_hash).transpose()?;
    Ok(VoiceParticipant {
        id,
        display_name,
        avatar_hash,
        speaking: state.speaking,
        muted: state.mute || state.voice_state.mute || state.voice_state.self_mute,
        deafened: state.voice_state.deaf
            || state.voice_state.self_deaf
            || state.voice_state.suppress,
    })
}

fn parse_deleted(data: Value) -> Result<DiscordRpcEvent, RpcParseError> {
    let deletion: WireDeletedUser = parse_data(data)?;
    Ok(DiscordRpcEvent::ParticipantDeleted {
        user_id: valid_id(deletion.user.id)?,
    })
}

fn parse_speaking(data: Value, speaking: bool) -> Result<DiscordRpcEvent, RpcParseError> {
    let event: WireSpeaking = parse_data(data)?;
    Ok(DiscordRpcEvent::SpeakingChanged {
        user_id: valid_id(event.user_id)?,
        speaking,
    })
}

fn parse_subscription(
    data: Value,
    nonce: Option<String>,
    subscribed: bool,
) -> Result<DiscordRpcEvent, RpcParseError> {
    let nonce = nonce.ok_or(RpcParseError::InvalidData)?;
    let subscription: WireSubscription = parse_data(data)?;
    let event =
        VoiceSubscriptionEvent::parse(&subscription.evt).ok_or(RpcParseError::InvalidData)?;
    Ok(DiscordRpcEvent::SubscriptionChanged {
        subscribed,
        event,
        nonce,
    })
}

fn parse_data<T: for<'de> Deserialize<'de>>(data: Value) -> Result<T, RpcParseError> {
    serde_json::from_value(data).map_err(|_| RpcParseError::InvalidData)
}

fn valid_id(value: String) -> Result<String, RpcParseError> {
    if !value.is_empty()
        && value.len() <= DISCORD_ID_MAX_BYTES
        && value.bytes().all(|byte| byte.is_ascii_digit())
    {
        Ok(value)
    } else {
        Err(RpcParseError::InvalidData)
    }
}

fn valid_name(value: String) -> Result<String, RpcParseError> {
    if !value.is_empty()
        && value.chars().count() <= DISCORD_NAME_MAX_CHARS
        && !value.chars().any(char::is_control)
    {
        Ok(value)
    } else {
        Err(RpcParseError::InvalidData)
    }
}

fn valid_avatar_hash(value: String) -> Result<String, RpcParseError> {
    if !value.is_empty()
        && value.len() <= DISCORD_AVATAR_HASH_MAX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        Ok(value)
    } else {
        Err(RpcParseError::InvalidData)
    }
}

fn valid_sensitive(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn valid_bounded_ascii(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && value.bytes().all(|byte| byte.is_ascii())
}
