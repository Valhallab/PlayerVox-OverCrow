pub const MEDIA_ERROR_MAX_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum MediaPlaybackStatus {
    Playing,
    Paused,
    #[default]
    Stopped,
}

impl MediaPlaybackStatus {
    pub(crate) fn priority(self) -> u8 {
        match self {
            Self::Playing => 2,
            Self::Paused => 1,
            Self::Stopped => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaCapabilities {
    pub can_go_previous: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaSnapshot {
    pub bus_name: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub playback_status: MediaPlaybackStatus,
    pub capabilities: MediaCapabilities,
    pub error: Option<String>,
}

impl MediaSnapshot {
    pub fn provider_error(message: &str) -> Self {
        Self {
            error: bound_nonempty_text(message, MEDIA_ERROR_MAX_BYTES),
            ..Self::default()
        }
    }

    pub(crate) fn supports(&self, action: MediaAction) -> bool {
        if self.bus_name.is_none() {
            return false;
        }

        match action {
            MediaAction::Previous => self.capabilities.can_go_previous,
            MediaAction::PlayPause => match self.playback_status {
                MediaPlaybackStatus::Playing => self.capabilities.can_pause,
                MediaPlaybackStatus::Paused | MediaPlaybackStatus::Stopped => {
                    self.capabilities.can_play
                }
            },
            MediaAction::Next => self.capabilities.can_go_next,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaAction {
    Previous,
    PlayPause,
    Next,
}

impl MediaAction {
    pub fn command_for(self, snapshot: &MediaSnapshot) -> Option<MediaCommand> {
        snapshot.supports(self).then(|| MediaCommand {
            bus_name: snapshot
                .bus_name
                .clone()
                .expect("supported player has a bus name"),
            action: self,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaCommand {
    bus_name: String,
    action: MediaAction,
}

impl MediaCommand {
    pub fn bus_name(&self) -> &str {
        &self.bus_name
    }

    pub(crate) fn action(&self) -> MediaAction {
        self.action
    }
}

pub(crate) fn bound_nonempty_text(value: &str, maximum_bytes: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let mut boundary = value.len().min(maximum_bytes);
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (boundary > 0).then(|| value[..boundary].to_owned())
}
