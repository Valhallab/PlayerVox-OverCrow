use std::fmt;

use zeroize::Zeroizing;

#[derive(Clone, Eq, PartialEq)]
pub struct SensitiveValue(Zeroizing<String>);

impl SensitiveValue {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SensitiveValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveValue([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceParticipant {
    pub id: String,
    pub display_name: String,
    pub avatar_hash: Option<String>,
    pub speaking: bool,
    pub muted: bool,
    pub deafened: bool,
}

impl VoiceParticipant {
    #[cfg(test)]
    pub fn for_test(id: &str, display_name: &str, speaking: bool) -> Self {
        Self {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            avatar_hash: None,
            speaking,
            muted: false,
            deafened: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceChannel {
    pub id: String,
    pub name: String,
    pub participants: Vec<VoiceParticipant>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VoiceSubscriptionEvent {
    VoiceChannelSelect,
    SpeakingStart,
    SpeakingStop,
    VoiceStateCreate,
    VoiceStateDelete,
    VoiceStateUpdate,
}

impl VoiceSubscriptionEvent {
    pub const CHANNEL_SCOPED: [Self; 5] = [
        Self::SpeakingStart,
        Self::SpeakingStop,
        Self::VoiceStateCreate,
        Self::VoiceStateDelete,
        Self::VoiceStateUpdate,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::VoiceChannelSelect => "VOICE_CHANNEL_SELECT",
            Self::SpeakingStart => "SPEAKING_START",
            Self::SpeakingStop => "SPEAKING_STOP",
            Self::VoiceStateCreate => "VOICE_STATE_CREATE",
            Self::VoiceStateDelete => "VOICE_STATE_DELETE",
            Self::VoiceStateUpdate => "VOICE_STATE_UPDATE",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        std::iter::once(Self::VoiceChannelSelect)
            .chain(Self::CHANNEL_SCOPED)
            .find(|event| event.as_str() == value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscordRpcEvent {
    Ready,
    AuthorizationGranted {
        nonce: String,
        code: SensitiveValue,
    },
    Authenticated {
        nonce: String,
        user_id: String,
    },
    ChannelSelected(Option<String>),
    ChannelSnapshot {
        nonce: String,
        channel: Option<VoiceChannel>,
    },
    ParticipantCreated(VoiceParticipant),
    ParticipantUpdated(VoiceParticipant),
    ParticipantDeleted {
        user_id: String,
    },
    SpeakingChanged {
        user_id: String,
        speaking: bool,
    },
    SubscriptionChanged {
        subscribed: bool,
        event: VoiceSubscriptionEvent,
        nonce: String,
    },
    ProviderError {
        command: Option<String>,
        code: i64,
        nonce: Option<String>,
    },
    Ignored,
}

pub fn sort_participants(participants: &mut [VoiceParticipant], local_user_id: Option<&str>) {
    participants.sort_by(|left, right| {
        participant_rank(left, local_user_id)
            .cmp(&participant_rank(right, local_user_id))
            .then_with(|| {
                left.display_name
                    .to_lowercase()
                    .cmp(&right.display_name.to_lowercase())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn participant_rank(participant: &VoiceParticipant, local_user_id: Option<&str>) -> u8 {
    match (
        local_user_id == Some(participant.id.as_str()),
        participant.speaking,
    ) {
        (true, _) => 0,
        (false, true) => 1,
        (false, false) => 2,
    }
}
