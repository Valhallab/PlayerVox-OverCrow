use std::{error::Error, fmt};

use serde::{Deserialize, Deserializer, Serialize, de};

pub const WIDGET_SCHEMA_VERSION: u32 = 3;
pub const DISCORD_PARTICIPANT_LIMIT_MIN: u8 = 2;
pub const DISCORD_PARTICIPANT_LIMIT_MAX: u8 = 16;
const LEGACY_DISCORD_AVATAR_SIZE_MIN: u16 = 24;
const LEGACY_DISCORD_AVATAR_SIZE_MAX: u16 = 64;
pub const WIDGET_SCALE_MIN: f32 = 0.75;
pub const WIDGET_SCALE_MAX: f32 = 1.75;
/// Minimum panel width (comfortable for Warframe panels without crushing text).
pub const WIDGET_PANEL_MIN: f32 = 280.0;
pub const WIDGET_PANEL_MIN_HEIGHT: f32 = 160.0;
pub const WIDGET_PANEL_MAX: f32 = 900.0;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WidgetId {
    Session,
    Clock,
    Performance,
    ManualStopwatch,
    Media,
    Notes,
    WarframeStatus,
    WarframeFissures,
    WarframeMarket,
    WarframeSortie,
    WarframeInvasions,
    TwitchChat,
    DiscordVoice,
}

impl WidgetId {
    pub const COUNT: usize = 13;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Session,
        Self::Clock,
        Self::Performance,
        Self::ManualStopwatch,
        Self::Media,
        Self::Notes,
        Self::WarframeStatus,
        Self::WarframeFissures,
        Self::WarframeMarket,
        Self::WarframeSortie,
        Self::WarframeInvasions,
        Self::TwitchChat,
        Self::DiscordVoice,
    ];

    pub const fn default_position(self) -> WidgetPosition {
        match self {
            Self::Session => WidgetPosition { x: 0.0, y: 0.0 },
            Self::Clock => WidgetPosition { x: 1.0, y: 0.0 },
            Self::Performance => WidgetPosition { x: 0.0, y: 1.0 },
            Self::ManualStopwatch => WidgetPosition { x: 1.0, y: 1.0 },
            Self::Media => WidgetPosition { x: 0.5, y: 0.0 },
            Self::Notes => WidgetPosition { x: 0.5, y: 1.0 },
            Self::WarframeStatus => WidgetPosition { x: 0.5, y: 0.12 },
            Self::WarframeFissures => WidgetPosition { x: 1.0, y: 0.45 },
            Self::WarframeMarket => WidgetPosition { x: 0.0, y: 0.45 },
            Self::WarframeSortie => WidgetPosition { x: 0.0, y: 0.18 },
            Self::WarframeInvasions => WidgetPosition { x: 1.0, y: 0.72 },
            Self::TwitchChat => WidgetPosition { x: 1.0, y: 0.28 },
            Self::DiscordVoice => WidgetPosition { x: 0.0, y: 0.28 },
        }
    }

    /// Default panel size in logical pixels (0 = auto-size from content).
    pub const fn default_panel_size(self) -> (f32, f32) {
        match self {
            Self::WarframeStatus => (380.0, 240.0),
            Self::WarframeFissures => (440.0, 340.0),
            Self::WarframeMarket => (400.0, 420.0),
            Self::WarframeSortie => (400.0, 300.0),
            Self::WarframeInvasions => (440.0, 360.0),
            Self::TwitchChat => (420.0, 360.0),
            Self::DiscordVoice => (280.0, 160.0),
            Self::Notes => (360.0, 280.0),
            Self::Media => (320.0, 160.0),
            _ => (0.0, 0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetPosition {
    pub x: f32,
    pub y: f32,
}

impl WidgetPosition {
    pub(crate) fn is_valid(self) -> bool {
        valid_ratio(self.x) && valid_ratio(self.y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetSettings {
    pub enabled: bool,
    pub show_in_passive: bool,
    pub position: WidgetPosition,
    /// UI scale for this widget (`1.0` = default).
    #[serde(default = "default_scale")]
    pub scale: f32,
    /// Preferred panel width in logical pixels (`0` = auto).
    #[serde(default)]
    pub width: f32,
    /// Preferred panel height in logical pixels (`0` = auto).
    #[serde(default)]
    pub height: f32,
    /// When true, the panel fill and border are omitted (content only).
    #[serde(default)]
    pub transparent_background: bool,
}

fn default_scale() -> f32 {
    1.0
}

impl WidgetSettings {
    const fn new(id: WidgetId, enabled: bool) -> Self {
        Self::with_passive(id, enabled, false)
    }

    const fn with_passive(id: WidgetId, enabled: bool, show_in_passive: bool) -> Self {
        let (width, height) = id.default_panel_size();
        Self {
            enabled,
            show_in_passive,
            position: id.default_position(),
            scale: 1.0,
            width,
            height,
            transparent_background: false,
        }
    }

    pub fn effective_panel_size(self, id: WidgetId) -> (f32, f32) {
        let (default_w, default_h) = id.default_panel_size();
        let width = if self.width > 0.0 {
            self.width
        } else {
            default_w
        };
        let height = if self.height > 0.0 {
            self.height
        } else {
            default_h
        };
        (width, height)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockDisplaySettings {
    pub show_date: bool,
}

impl Default for ClockDisplaySettings {
    fn default() -> Self {
        Self { show_date: true }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceLayout {
    Vertical,
    #[default]
    Horizontal,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceDisplaySettings {
    pub layout: PerformanceLayout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotesDisplaySettings {
    pub show_note: bool,
    pub show_checklist: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscordVoiceAlignment {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DiscordVoiceDisplaySettings {
    pub participant_limit: u8,
    pub alignment: DiscordVoiceAlignment,
}

impl Default for DiscordVoiceDisplaySettings {
    fn default() -> Self {
        Self {
            participant_limit: 8,
            alignment: DiscordVoiceAlignment::Left,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscordVoiceDisplayWire {
    participant_limit: u8,
    #[serde(default)]
    alignment: DiscordVoiceAlignment,
    #[serde(default)]
    avatar_size: Option<u16>,
}

impl<'de> Deserialize<'de> for DiscordVoiceDisplaySettings {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        let wire = DiscordVoiceDisplayWire::deserialize(deserializer)?;
        if wire.avatar_size.is_some_and(|size| {
            !(LEGACY_DISCORD_AVATAR_SIZE_MIN..=LEGACY_DISCORD_AVATAR_SIZE_MAX).contains(&size)
        }) {
            return Err(de::Error::custom("invalid legacy Discord avatar size"));
        }
        Ok(Self {
            participant_limit: wire.participant_limit,
            alignment: wire.alignment,
        })
    }
}

impl Default for NotesDisplaySettings {
    fn default() -> Self {
        Self {
            show_note: true,
            show_checklist: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetProfile {
    pub schema_version: u32,
    pub session: WidgetSettings,
    pub clock: WidgetSettings,
    #[serde(default)]
    pub clock_display: ClockDisplaySettings,
    pub performance: WidgetSettings,
    #[serde(default)]
    pub performance_display: PerformanceDisplaySettings,
    pub manual_stopwatch: WidgetSettings,
    pub media: WidgetSettings,
    pub notes: WidgetSettings,
    #[serde(default)]
    pub notes_display: NotesDisplaySettings,
    #[serde(default = "default_warframe_status")]
    pub warframe_status: WidgetSettings,
    #[serde(default = "default_warframe_fissures")]
    pub warframe_fissures: WidgetSettings,
    #[serde(default = "default_warframe_market")]
    pub warframe_market: WidgetSettings,
    #[serde(default = "default_warframe_sortie")]
    pub warframe_sortie: WidgetSettings,
    #[serde(default = "default_warframe_invasions")]
    pub warframe_invasions: WidgetSettings,
    #[serde(default = "default_twitch_chat")]
    pub twitch_chat: WidgetSettings,
    #[serde(default = "default_discord_voice")]
    pub discord_voice: WidgetSettings,
    #[serde(default)]
    pub discord_voice_display: DiscordVoiceDisplaySettings,
    /// Legacy field kept so older `widgets.json` files still load.
    #[serde(default = "default_legacy_warframe_nightwave", skip_serializing)]
    pub warframe_nightwave: WidgetSettings,
}

fn default_warframe_status() -> WidgetSettings {
    WidgetSettings::with_passive(WidgetId::WarframeStatus, false, true)
}

fn default_warframe_fissures() -> WidgetSettings {
    WidgetSettings::with_passive(WidgetId::WarframeFissures, false, true)
}

fn default_warframe_market() -> WidgetSettings {
    WidgetSettings::with_passive(WidgetId::WarframeMarket, false, false)
}

fn default_warframe_sortie() -> WidgetSettings {
    WidgetSettings::with_passive(WidgetId::WarframeSortie, false, true)
}

fn default_warframe_invasions() -> WidgetSettings {
    WidgetSettings::with_passive(WidgetId::WarframeInvasions, false, true)
}

fn default_twitch_chat() -> WidgetSettings {
    WidgetSettings::with_passive(WidgetId::TwitchChat, false, false)
}

fn default_discord_voice() -> WidgetSettings {
    WidgetSettings::with_passive(WidgetId::DiscordVoice, false, true)
}

fn default_legacy_warframe_nightwave() -> WidgetSettings {
    // Nightwave widget removed; keep a passive-disabled stub for old configs.
    WidgetSettings::with_passive(WidgetId::WarframeStatus, false, true)
}

impl Default for WidgetProfile {
    fn default() -> Self {
        Self {
            schema_version: WIDGET_SCHEMA_VERSION,
            session: WidgetSettings::new(WidgetId::Session, true),
            clock: WidgetSettings::new(WidgetId::Clock, false),
            clock_display: ClockDisplaySettings::default(),
            performance: WidgetSettings::new(WidgetId::Performance, false),
            performance_display: PerformanceDisplaySettings::default(),
            manual_stopwatch: WidgetSettings::new(WidgetId::ManualStopwatch, false),
            media: WidgetSettings::new(WidgetId::Media, false),
            notes: WidgetSettings::new(WidgetId::Notes, false),
            notes_display: NotesDisplaySettings::default(),
            warframe_status: default_warframe_status(),
            warframe_fissures: default_warframe_fissures(),
            warframe_market: default_warframe_market(),
            warframe_sortie: default_warframe_sortie(),
            warframe_invasions: default_warframe_invasions(),
            twitch_chat: default_twitch_chat(),
            discord_voice: default_discord_voice(),
            discord_voice_display: DiscordVoiceDisplaySettings::default(),
            warframe_nightwave: default_legacy_warframe_nightwave(),
        }
    }
}

impl WidgetProfile {
    pub fn settings(&self, id: WidgetId) -> &WidgetSettings {
        match id {
            WidgetId::Session => &self.session,
            WidgetId::Clock => &self.clock,
            WidgetId::Performance => &self.performance,
            WidgetId::ManualStopwatch => &self.manual_stopwatch,
            WidgetId::Media => &self.media,
            WidgetId::Notes => &self.notes,
            WidgetId::WarframeStatus => &self.warframe_status,
            WidgetId::WarframeFissures => &self.warframe_fissures,
            WidgetId::WarframeMarket => &self.warframe_market,
            WidgetId::WarframeSortie => &self.warframe_sortie,
            WidgetId::WarframeInvasions => &self.warframe_invasions,
            WidgetId::TwitchChat => &self.twitch_chat,
            WidgetId::DiscordVoice => &self.discord_voice,
        }
    }

    pub fn settings_mut(&mut self, id: WidgetId) -> &mut WidgetSettings {
        match id {
            WidgetId::Session => &mut self.session,
            WidgetId::Clock => &mut self.clock,
            WidgetId::Performance => &mut self.performance,
            WidgetId::ManualStopwatch => &mut self.manual_stopwatch,
            WidgetId::Media => &mut self.media,
            WidgetId::Notes => &mut self.notes,
            WidgetId::WarframeStatus => &mut self.warframe_status,
            WidgetId::WarframeFissures => &mut self.warframe_fissures,
            WidgetId::WarframeMarket => &mut self.warframe_market,
            WidgetId::WarframeSortie => &mut self.warframe_sortie,
            WidgetId::WarframeInvasions => &mut self.warframe_invasions,
            WidgetId::TwitchChat => &mut self.twitch_chat,
            WidgetId::DiscordVoice => &mut self.discord_voice,
        }
    }

    pub fn validate(self) -> Result<Self, WidgetProfileError> {
        if self.schema_version != WIDGET_SCHEMA_VERSION {
            return Err(WidgetProfileError::UnsupportedSchemaVersion);
        }
        for id in WidgetId::ALL {
            let settings = self.settings(id);
            if !settings.position.is_valid() {
                return Err(WidgetProfileError::InvalidPosition(id));
            }
            if !valid_scale(settings.scale) {
                return Err(WidgetProfileError::InvalidScale(id));
            }
            if !valid_panel_dim(settings.width) || !valid_panel_dim(settings.height) {
                return Err(WidgetProfileError::InvalidSize(id));
            }
        }
        if !self.notes_display.show_note && !self.notes_display.show_checklist {
            return Err(WidgetProfileError::EmptyNotesDisplay);
        }
        if !(DISCORD_PARTICIPANT_LIMIT_MIN..=DISCORD_PARTICIPANT_LIMIT_MAX)
            .contains(&self.discord_voice_display.participant_limit)
        {
            return Err(WidgetProfileError::InvalidDiscordParticipantLimit);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidgetProfileError {
    UnsupportedSchemaVersion,
    InvalidPosition(WidgetId),
    InvalidScale(WidgetId),
    InvalidSize(WidgetId),
    EmptyNotesDisplay,
    InvalidDiscordParticipantLimit,
}

impl fmt::Display for WidgetProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion => {
                formatter.write_str("unsupported widget profile schema version")
            }
            Self::InvalidPosition(id) => {
                write!(formatter, "invalid normalized position for widget {id:?}")
            }
            Self::InvalidScale(id) => {
                write!(formatter, "invalid scale for widget {id:?}")
            }
            Self::InvalidSize(id) => {
                write!(formatter, "invalid panel size for widget {id:?}")
            }
            Self::EmptyNotesDisplay => {
                formatter.write_str("notes widget must show a note or checklist")
            }
            Self::InvalidDiscordParticipantLimit => {
                formatter.write_str("invalid Discord participant limit")
            }
        }
    }
}

impl Error for WidgetProfileError {}

fn valid_ratio(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn valid_scale(value: f32) -> bool {
    value.is_finite() && (WIDGET_SCALE_MIN..=WIDGET_SCALE_MAX).contains(&value)
}

fn valid_panel_dim(value: f32) -> bool {
    // Shared max; min is the height floor so auto (0) and either axis remain valid.
    value.is_finite()
        && (value == 0.0 || (WIDGET_PANEL_MIN_HEIGHT..=WIDGET_PANEL_MAX).contains(&value))
}
