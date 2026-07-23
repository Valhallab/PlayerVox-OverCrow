use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

pub const WIDGET_SCHEMA_VERSION: u32 = 1;
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
}

impl WidgetId {
    pub const ALL: [Self; 11] = [
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetProfile {
    pub schema_version: u32,
    pub session: WidgetSettings,
    pub clock: WidgetSettings,
    pub performance: WidgetSettings,
    pub manual_stopwatch: WidgetSettings,
    pub media: WidgetSettings,
    pub notes: WidgetSettings,
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
            performance: WidgetSettings::new(WidgetId::Performance, false),
            manual_stopwatch: WidgetSettings::new(WidgetId::ManualStopwatch, false),
            media: WidgetSettings::new(WidgetId::Media, false),
            notes: WidgetSettings::new(WidgetId::Notes, false),
            warframe_status: default_warframe_status(),
            warframe_fissures: default_warframe_fissures(),
            warframe_market: default_warframe_market(),
            warframe_sortie: default_warframe_sortie(),
            warframe_invasions: default_warframe_invasions(),
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
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidgetProfileError {
    UnsupportedSchemaVersion,
    InvalidPosition(WidgetId),
    InvalidScale(WidgetId),
    InvalidSize(WidgetId),
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
