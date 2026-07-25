use std::{collections::VecDeque, time::Instant};

pub const TWITCH_MESSAGE_BUFFER_MAX: usize = 200;
pub const TWITCH_PENDING_SEND_MAX: usize = 8;
pub const TWITCH_MESSAGE_MAX_CHARS: usize = 500;
pub const TWITCH_DISPLAY_NAME_MAX_CHARS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TwitchConnectionState {
    Inert,
    Disconnected,
    Authorizing,
    Connecting,
    Joined,
    Reconnecting,
    Failed(TwitchFailureCategory),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TwitchFailureCategory {
    Authentication,
    AuthorizationExpired,
    ChannelUnavailable,
    Connection,
    RateLimited,
    ProviderResponse,
    CredentialStore,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceAuthorization {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TwitchReplyContext {
    pub message_id: String,
    pub display_name: String,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedChatMessage {
    pub id: String,
    pub display_name: String,
    pub name_color: Option<[u8; 3]>,
    pub text: String,
    pub reply: Option<TwitchReplyContext>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TwitchSendState {
    Received,
    Pending,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TwitchMessage {
    pub id: String,
    pub display_name: String,
    pub name_color: Option<[u8; 3]>,
    pub text: String,
    pub reply: Option<TwitchReplyContext>,
    pub received_at: Instant,
    pub client_nonce: Option<String>,
    pub send_state: TwitchSendState,
}

impl TwitchMessage {
    pub fn received(parsed: ParsedChatMessage, received_at: Instant) -> Self {
        Self {
            id: parsed.id,
            display_name: parsed.display_name,
            name_color: parsed.name_color,
            text: parsed.text,
            reply: parsed.reply,
            received_at,
            client_nonce: None,
            send_state: TwitchSendState::Received,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TwitchSnapshot {
    pub generation: u64,
    pub channel: Option<String>,
    pub connection: TwitchConnectionState,
    pub messages: Vec<TwitchMessage>,
    pub authorization: Option<DeviceAuthorization>,
    pub authenticated_login: Option<String>,
    pub credentials_available: bool,
    pub credentials_persisted: bool,
    pub client_configured: bool,
    pub send_receipt: Option<TwitchSendReceipt>,
}

impl Default for TwitchSnapshot {
    fn default() -> Self {
        Self {
            generation: 0,
            channel: None,
            connection: TwitchConnectionState::Inert,
            messages: Vec::new(),
            authorization: None,
            authenticated_login: None,
            credentials_available: false,
            credentials_persisted: false,
            client_configured: false,
            send_receipt: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TwitchSendReceiptState {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TwitchSendReceipt {
    pub request_id: u64,
    pub state: TwitchSendReceiptState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TwitchCommand {
    BeginAuthentication,
    CancelAuthentication,
    /// Close chat while keeping the stored Twitch session.
    Disconnect,
    /// Revoke tokens and forget the Twitch account (full sign-out).
    SignOut,
    Reconnect,
    SendMessage {
        request_id: u64,
        generation: u64,
        channel: String,
        text: String,
        reply_to: Option<String>,
    },
}

#[derive(Default)]
pub struct MessageBuffer {
    messages: VecDeque<TwitchMessage>,
}

impl MessageBuffer {
    fn push(&mut self, message: TwitchMessage) {
        if self.messages.len() == TWITCH_MESSAGE_BUFFER_MAX {
            self.messages.pop_front();
        }
        self.messages.push_back(message);
    }

    pub fn push_pending(
        &mut self,
        nonce: String,
        display_name: String,
        text: String,
        received_at: Instant,
    ) -> bool {
        let pending_count = self
            .messages
            .iter()
            .filter(|message| message.send_state == TwitchSendState::Pending)
            .count();
        if pending_count >= TWITCH_PENDING_SEND_MAX {
            return false;
        }
        self.push(TwitchMessage {
            id: String::new(),
            display_name,
            name_color: None,
            text,
            reply: None,
            received_at,
            client_nonce: Some(nonce),
            send_state: TwitchSendState::Pending,
        });
        true
    }

    pub fn fail_pending(&mut self, nonce: &str) {
        if let Some(message) = self.messages.iter_mut().find(|message| {
            message.send_state == TwitchSendState::Pending
                && message.client_nonce.as_deref() == Some(nonce)
        }) {
            message.send_state = TwitchSendState::Failed;
        }
    }

    pub fn mark_sent(&mut self, nonce: &str, message_id: String) {
        if let Some(message) = self.messages.iter_mut().find(|message| {
            message.send_state == TwitchSendState::Pending
                && message.client_nonce.as_deref() == Some(nonce)
        }) {
            message.send_state = TwitchSendState::Received;
            message.id = message_id;
        }
    }

    pub fn upsert_received(&mut self, parsed: ParsedChatMessage, received_at: Instant) {
        let preserved_nonce = self
            .messages
            .iter()
            .find(|message| message.id == parsed.id)
            .and_then(|message| message.client_nonce.clone());
        let id = parsed.id.clone();
        let mut received = TwitchMessage::received(parsed, received_at);
        received.client_nonce = preserved_nonce;
        if let Some(index) = self.messages.iter().position(|message| message.id == id) {
            self.messages[index] = received;
        } else {
            self.push(received);
        }
    }

    pub fn remove_message(&mut self, message_id: &str) {
        self.messages.retain(|message| message.id != message_id);
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    pub fn snapshot(&self) -> Vec<TwitchMessage> {
        self.messages.iter().cloned().collect()
    }
}
