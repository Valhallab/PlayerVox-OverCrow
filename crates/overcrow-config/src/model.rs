use std::{collections::BTreeSet, error::Error, fmt, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutSettings {
    pub enabled: bool,
    pub accelerator: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualGame {
    pub id: String,
    pub name: String,
    pub executable: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleSettings {
    pub schema_version: u32,
    pub enabled: bool,
    #[serde(deserialize_with = "deserialize_unique_steam_app_ids")]
    pub selected_steam_app_ids: BTreeSet<u32>,
    pub manual_games: Vec<ManualGame>,
    pub shortcut: ShortcutSettings,
}

impl Default for LifecycleSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            enabled: false,
            selected_steam_app_ids: BTreeSet::new(),
            manual_games: Vec::new(),
            shortcut: ShortcutSettings {
                enabled: true,
                accelerator: "Meta+Alt+O".to_owned(),
            },
        }
    }
}

impl LifecycleSettings {
    pub fn validate(self) -> Result<Self, SettingsError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::UnsupportedSchemaVersion);
        }
        if self.selected_steam_app_ids.contains(&0) {
            return Err(SettingsError::ZeroSteamAppId);
        }

        let mut manual_ids = BTreeSet::new();
        for game in &self.manual_games {
            if !manual_ids.insert(game.id.as_str()) {
                return Err(SettingsError::DuplicateManualId);
            }
            if !game.executable.is_absolute() {
                return Err(SettingsError::NonAbsoluteExecutable);
            }
        }

        if !accelerator_is_valid(&self.shortcut.accelerator) {
            return Err(SettingsError::MalformedAccelerator);
        }

        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsError {
    UnsupportedSchemaVersion,
    ZeroSteamAppId,
    DuplicateManualId,
    NonAbsoluteExecutable,
    MalformedAccelerator,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedSchemaVersion => "unsupported settings schema version",
            Self::ZeroSteamAppId => "Steam app ID must not be zero",
            Self::DuplicateManualId => "manual game IDs must be unique",
            Self::NonAbsoluteExecutable => "manual game executable must be absolute",
            Self::MalformedAccelerator => "shortcut accelerator is malformed",
        };
        formatter.write_str(message)
    }
}

impl Error for SettingsError {}

fn deserialize_unique_steam_app_ids<'de, D>(deserializer: D) -> Result<BTreeSet<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    let ids = Vec::<u32>::deserialize(deserializer)?;
    let mut unique = BTreeSet::new();
    for id in ids {
        if !unique.insert(id) {
            return Err(D::Error::custom(format_args!(
                "duplicate Steam app ID {id}"
            )));
        }
    }
    Ok(unique)
}

fn accelerator_is_valid(accelerator: &str) -> bool {
    let mut parts = accelerator.split('+').peekable();
    let mut modifiers = BTreeSet::new();
    let mut modifier_count = 0;
    let mut previous_rank = None;

    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            let mut characters = part.chars();
            return modifier_count > 0
                && characters
                    .next()
                    .is_some_and(|character| character.is_ascii_alphanumeric())
                && characters.next().is_none();
        }

        let rank = match part {
            "Meta" => 0,
            "Ctrl" => 1,
            "Alt" => 2,
            "Shift" => 3,
            _ => return false,
        };
        if previous_rank.is_some_and(|previous| rank <= previous) || !modifiers.insert(part) {
            return false;
        }
        previous_rank = Some(rank);
        modifier_count += 1;
    }

    false
}
