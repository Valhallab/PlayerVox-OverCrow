use std::{
    env,
    error::Error,
    ffi::OsStr,
    fmt, fs,
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::CommittedSettingsSaveError;

pub const WARFRAME_PREFS_SCHEMA_VERSION: u32 = 1;
pub const WARFRAME_STEAM_APP_ID: u32 = 230_410;
pub const WARFRAME_PREFS_MAX_BYTES: usize = 64 * 1024;
pub const WARFRAME_MARKET_QUERY_MAX_CHARS: usize = 64;
pub const WARFRAME_INVASION_WATCHLIST_MAX: usize = 24;
pub const WARFRAME_INVASION_WATCHLIST_ENTRY_MAX_CHARS: usize = 96;
pub const WARFRAME_ACTIVITY_DONE_MAX: usize = 128;
pub const WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS: usize = 96;

const OPEN_FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FissureEra {
    Lith,
    Meso,
    Neo,
    Axi,
    Requiem,
    Omni,
}

impl FissureEra {
    pub const ALL: [Self; 6] = [
        Self::Lith,
        Self::Meso,
        Self::Neo,
        Self::Axi,
        Self::Requiem,
        Self::Omni,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Lith => "Lith",
            Self::Meso => "Meso",
            Self::Neo => "Neo",
            Self::Axi => "Axi",
            Self::Requiem => "Requiem",
            Self::Omni => "Omni",
        }
    }

    pub fn from_void_modifier(modifier: &str) -> Option<Self> {
        match modifier {
            "VoidT1" => Some(Self::Lith),
            "VoidT2" => Some(Self::Meso),
            "VoidT3" => Some(Self::Neo),
            "VoidT4" => Some(Self::Axi),
            "VoidT5" => Some(Self::Requiem),
            "VoidT6" => Some(Self::Omni),
            _ => None,
        }
    }
}

fn default_true() -> bool {
    true
}

fn normalize_string_list(
    entries: Vec<String>,
    max_chars: usize,
    max_len: usize,
    too_long_entry: WarframePrefsError,
    invalid_entry: WarframePrefsError,
    too_long_list: WarframePrefsError,
) -> Result<Vec<String>, WarframePrefsError> {
    let mut out = Vec::new();
    for entry in entries {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() > max_chars {
            return Err(too_long_entry);
        }
        if trimmed.chars().any(|c| c.is_control() || c == '\0') {
            return Err(invalid_entry);
        }
        if !out.iter().any(|existing: &String| existing == trimmed) {
            out.push(trimmed.to_owned());
        }
        if out.len() > max_len {
            return Err(too_long_list);
        }
    }
    Ok(out)
}

/// Last path segment of a Lotus uniqueName (or the whole string if no `/`).
pub fn path_tail(path: &str) -> String {
    path.trim()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim()
        .to_owned()
}

/// Which open-world / utility rows appear on the status widget.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum StatusRow {
    Cetus,
    Vallis,
    Cambion,
    Zariman,
    DailyReset,
    Baro,
}

impl StatusRow {
    pub const ALL: [Self; 6] = [
        Self::Cetus,
        Self::Vallis,
        Self::Cambion,
        Self::Zariman,
        Self::DailyReset,
        Self::Baro,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cetus => "Cetus",
            Self::Vallis => "Vallis",
            Self::Cambion => "Cambion",
            Self::Zariman => "Zariman",
            Self::DailyReset => "Reset",
            Self::Baro => "Baro",
        }
    }

    /// Matches cycle ids from worldstate parse (`cetus`, `vallis`, …).
    pub fn from_cycle_id(id: &str) -> Option<Self> {
        match id {
            "cetus" => Some(Self::Cetus),
            "vallis" => Some(Self::Vallis),
            "cambion" => Some(Self::Cambion),
            "zariman" => Some(Self::Zariman),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarframePrefs {
    pub schema_version: u32,
    /// Empty means all eras are shown.
    pub fissure_eras: Vec<FissureEra>,
    /// Show normal (non-SP, non-Railjack) star-chart fissures.
    #[serde(default = "default_true")]
    pub show_normal: bool,
    /// Show Steel Path star-chart fissures.
    #[serde(default = "default_true")]
    pub show_steel_path: bool,
    /// Show Railjack void storms.
    #[serde(default = "default_true")]
    pub show_railjack: bool,
    /// Status widget: compact single horizontal bar.
    #[serde(default)]
    pub status_horizontal: bool,
    #[serde(default = "default_true")]
    pub show_status_cetus: bool,
    #[serde(default = "default_true")]
    pub show_status_vallis: bool,
    #[serde(default = "default_true")]
    pub show_status_cambion: bool,
    #[serde(default = "default_true")]
    pub show_status_zariman: bool,
    #[serde(default = "default_true")]
    pub show_status_reset: bool,
    #[serde(default = "default_true")]
    pub show_status_baro: bool,
    /// Include seconds on status widget timers.
    #[serde(default = "default_true")]
    pub show_status_seconds: bool,
    /// Include seconds on fissure row timers.
    #[serde(default = "default_true")]
    pub show_fissure_seconds: bool,
    /// Show planet / node labels on fissure rows.
    #[serde(default = "default_true")]
    pub show_fissure_node: bool,
    pub last_market_query: String,
    /// Hide invasions marked completed by DE.
    #[serde(default = "default_true")]
    pub invasion_hide_completed: bool,
    /// Sort user-finished invasions below open ones.
    #[serde(default = "default_true")]
    pub invasion_push_done_down: bool,
    /// One-line rows: checkbox + dual progress bar with rewards and node inside.
    #[serde(default)]
    pub invasion_compact: bool,
    /// Empty = all rewards. Otherwise only invasions offering one of these item keys.
    #[serde(default)]
    pub invasion_resource_filter: Vec<String>,
    /// Item path tails to highlight on the invasions widget.
    #[serde(default)]
    pub invasion_reward_watchlist: Vec<String>,
    /// Legacy Nightwave fields (widget removed). Kept so older `warframe.json` loads.
    #[serde(default, skip_serializing)]
    pub nightwave_daily_only: bool,
    #[serde(default, skip_serializing)]
    pub nightwave_show_expired: bool,
    /// Include seconds on Sortie/Archon timers.
    #[serde(default = "default_true")]
    pub show_activity_seconds: bool,
    /// Local completion marks (`sortie:…`, `archon:…`, `invasion:…`).
    #[serde(default)]
    pub activity_done: Vec<String>,
}

impl Default for WarframePrefs {
    fn default() -> Self {
        Self {
            schema_version: WARFRAME_PREFS_SCHEMA_VERSION,
            fissure_eras: Vec::new(),
            show_normal: true,
            show_steel_path: true,
            show_railjack: true,
            status_horizontal: false,
            show_status_cetus: true,
            show_status_vallis: true,
            show_status_cambion: true,
            show_status_zariman: true,
            show_status_reset: true,
            show_status_baro: true,
            show_status_seconds: true,
            show_fissure_seconds: true,
            show_fissure_node: true,
            last_market_query: String::new(),
            invasion_hide_completed: true,
            invasion_push_done_down: true,
            invasion_compact: false,
            invasion_resource_filter: Vec::new(),
            invasion_reward_watchlist: Vec::new(),
            nightwave_daily_only: false,
            nightwave_show_expired: false,
            show_activity_seconds: true,
            activity_done: Vec::new(),
        }
    }
}

impl WarframePrefs {
    pub fn validate(self) -> Result<Self, WarframePrefsError> {
        if self.schema_version != WARFRAME_PREFS_SCHEMA_VERSION {
            return Err(WarframePrefsError::UnsupportedSchemaVersion);
        }
        if self.last_market_query.chars().count() > WARFRAME_MARKET_QUERY_MAX_CHARS {
            return Err(WarframePrefsError::QueryTooLong);
        }
        if self
            .last_market_query
            .chars()
            .any(|c| c.is_control() || c == '\0')
        {
            return Err(WarframePrefsError::InvalidQuery);
        }
        // Deduplicate era lists (stable order preserved for first occurrence).
        let mut eras = Vec::new();
        for era in self.fissure_eras {
            if !eras.contains(&era) {
                eras.push(era);
            }
        }
        // At least one source category must remain enabled after validation.
        if !self.show_normal && !self.show_steel_path && !self.show_railjack {
            return Err(WarframePrefsError::NoSourceSelected);
        }
        let watchlist = normalize_string_list(
            self.invasion_reward_watchlist,
            WARFRAME_INVASION_WATCHLIST_ENTRY_MAX_CHARS,
            WARFRAME_INVASION_WATCHLIST_MAX,
            WarframePrefsError::WatchlistEntryTooLong,
            WarframePrefsError::InvalidWatchlistEntry,
            WarframePrefsError::WatchlistTooLong,
        )?;
        let resource_filter = normalize_string_list(
            self.invasion_resource_filter,
            WARFRAME_INVASION_WATCHLIST_ENTRY_MAX_CHARS,
            WARFRAME_INVASION_WATCHLIST_MAX,
            WarframePrefsError::WatchlistEntryTooLong,
            WarframePrefsError::InvalidWatchlistEntry,
            WarframePrefsError::WatchlistTooLong,
        )?;
        let activity_done = normalize_string_list(
            self.activity_done,
            WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS,
            WARFRAME_ACTIVITY_DONE_MAX,
            WarframePrefsError::ActivityDoneEntryTooLong,
            WarframePrefsError::InvalidActivityDoneEntry,
            WarframePrefsError::ActivityDoneTooLong,
        )?;
        Ok(Self {
            schema_version: self.schema_version,
            fissure_eras: eras,
            show_normal: self.show_normal,
            show_steel_path: self.show_steel_path,
            show_railjack: self.show_railjack,
            status_horizontal: self.status_horizontal,
            show_status_cetus: self.show_status_cetus,
            show_status_vallis: self.show_status_vallis,
            show_status_cambion: self.show_status_cambion,
            show_status_zariman: self.show_status_zariman,
            show_status_reset: self.show_status_reset,
            show_status_baro: self.show_status_baro,
            show_status_seconds: self.show_status_seconds,
            show_fissure_seconds: self.show_fissure_seconds,
            show_fissure_node: self.show_fissure_node,
            last_market_query: self.last_market_query,
            invasion_hide_completed: self.invasion_hide_completed,
            invasion_push_done_down: self.invasion_push_done_down,
            invasion_compact: self.invasion_compact,
            invasion_resource_filter: resource_filter,
            invasion_reward_watchlist: watchlist,
            nightwave_daily_only: false,
            nightwave_show_expired: false,
            show_activity_seconds: self.show_activity_seconds,
            activity_done,
        })
    }

    /// Empty filter means every resource is shown.
    pub fn invasion_resource_checked(&self, item_key: &str) -> bool {
        let key = path_tail(item_key);
        self.invasion_resource_filter.is_empty()
            || self
                .invasion_resource_filter
                .iter()
                .any(|entry| path_tail(entry) == key)
    }

    /// Toggle a resource in the invasion filter.
    /// `available` is the set of item keys currently offered by active invasions.
    pub fn toggle_invasion_resource_filter(&mut self, item_key: &str, available: &[String]) {
        let key = path_tail(item_key);
        if key.is_empty() {
            return;
        }
        let available_tails: Vec<String> = available
            .iter()
            .map(|entry| path_tail(entry))
            .filter(|entry| !entry.is_empty())
            .collect();

        if self.invasion_resource_filter.is_empty() {
            // Leaving "show all": keep every available resource except this one.
            self.invasion_resource_filter = available_tails
                .into_iter()
                .filter(|entry| entry != &key)
                .take(WARFRAME_INVASION_WATCHLIST_MAX)
                .collect();
            return;
        }

        if let Some(index) = self
            .invasion_resource_filter
            .iter()
            .position(|entry| path_tail(entry) == key)
        {
            self.invasion_resource_filter.remove(index);
            // Empty exclusive filter → fall back to show all.
            if self.invasion_resource_filter.is_empty() {
                return;
            }
        } else if self.invasion_resource_filter.len() < WARFRAME_INVASION_WATCHLIST_MAX
            && key.chars().count() <= WARFRAME_INVASION_WATCHLIST_ENTRY_MAX_CHARS
        {
            self.invasion_resource_filter.push(key.clone());
        }

        // If every available resource is selected, collapse back to "show all".
        if !available_tails.is_empty()
            && available_tails.iter().all(|available_key| {
                self.invasion_resource_filter
                    .iter()
                    .any(|entry| path_tail(entry) == *available_key)
            })
        {
            self.invasion_resource_filter.clear();
        }
    }

    pub fn invasion_matches_resource_filter(&self, invasion_keys: &[&str]) -> bool {
        if self.invasion_resource_filter.is_empty() {
            return true;
        }
        invasion_keys
            .iter()
            .any(|key| self.invasion_resource_checked(key))
    }

    pub fn activity_is_done(&self, key: &str) -> bool {
        let key = key.trim();
        !key.is_empty() && self.activity_done.iter().any(|entry| entry == key)
    }

    /// Retain completion state only for keys explicitly recognized by the current snapshot.
    pub fn prune_activity_done(&mut self, current_keys: &[String]) {
        self.activity_done
            .retain(|entry| current_keys.iter().any(|current_key| current_key == entry));
    }

    pub fn toggle_activity_done(&mut self, key: &str) {
        let key = key.trim();
        if key.is_empty() || key.chars().count() > WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS {
            return;
        }
        if let Some(index) = self.activity_done.iter().position(|entry| entry == key) {
            self.activity_done.remove(index);
        } else {
            if self.activity_done.len() >= WARFRAME_ACTIVITY_DONE_MAX {
                self.activity_done.remove(0);
            }
            self.activity_done.push(key.to_owned());
        }
    }

    pub fn set_activity_done(&mut self, key: &str, done: bool) {
        let is_done = self.activity_is_done(key);
        if done != is_done {
            self.toggle_activity_done(key);
        }
    }

    pub fn invasion_watchlisted(&self, item_key: &str) -> bool {
        let key = path_tail(item_key);
        self.invasion_reward_watchlist
            .iter()
            .any(|entry| path_tail(entry) == key)
    }

    pub fn toggle_invasion_watchlist(&mut self, item_key: &str) {
        let key = path_tail(item_key);
        if key.is_empty() {
            return;
        }
        if let Some(index) = self
            .invasion_reward_watchlist
            .iter()
            .position(|entry| path_tail(entry) == key)
        {
            self.invasion_reward_watchlist.remove(index);
        } else if self.invasion_reward_watchlist.len() < WARFRAME_INVASION_WATCHLIST_MAX
            && key.chars().count() <= WARFRAME_INVASION_WATCHLIST_ENTRY_MAX_CHARS
        {
            self.invasion_reward_watchlist.push(key);
        }
    }

    pub fn era_enabled(&self, era: FissureEra) -> bool {
        self.fissure_eras.is_empty() || self.fissure_eras.contains(&era)
    }

    pub fn source_enabled(&self, source: FissureSource) -> bool {
        match source {
            FissureSource::Normal => self.show_normal,
            FissureSource::SteelPath => self.show_steel_path,
            FissureSource::Railjack => self.show_railjack,
        }
    }

    pub fn status_row_visible(&self, row: StatusRow) -> bool {
        match row {
            StatusRow::Cetus => self.show_status_cetus,
            StatusRow::Vallis => self.show_status_vallis,
            StatusRow::Cambion => self.show_status_cambion,
            StatusRow::Zariman => self.show_status_zariman,
            StatusRow::DailyReset => self.show_status_reset,
            StatusRow::Baro => self.show_status_baro,
        }
    }

    pub fn set_status_row_visible(&mut self, row: StatusRow, visible: bool) {
        match row {
            StatusRow::Cetus => self.show_status_cetus = visible,
            StatusRow::Vallis => self.show_status_vallis = visible,
            StatusRow::Cambion => self.show_status_cambion = visible,
            StatusRow::Zariman => self.show_status_zariman = visible,
            StatusRow::DailyReset => self.show_status_reset = visible,
            StatusRow::Baro => self.show_status_baro = visible,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FissureSource {
    Normal,
    SteelPath,
    Railjack,
}

impl FissureSource {
    pub const ALL: [Self; 3] = [Self::Normal, Self::SteelPath, Self::Railjack];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::SteelPath => "Steel Path",
            Self::Railjack => "Railjack",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WarframePrefsError {
    UnsupportedSchemaVersion,
    QueryTooLong,
    InvalidQuery,
    NoSourceSelected,
    WatchlistTooLong,
    WatchlistEntryTooLong,
    InvalidWatchlistEntry,
    ActivityDoneTooLong,
    ActivityDoneEntryTooLong,
    InvalidActivityDoneEntry,
}

impl fmt::Display for WarframePrefsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion => {
                formatter.write_str("unsupported warframe prefs schema version")
            }
            Self::QueryTooLong => formatter.write_str("market query is too long"),
            Self::InvalidQuery => formatter.write_str("market query is invalid"),
            Self::NoSourceSelected => {
                formatter.write_str("at least one fissure source must be enabled")
            }
            Self::WatchlistTooLong => formatter.write_str("invasion watchlist is too long"),
            Self::WatchlistEntryTooLong => {
                formatter.write_str("invasion watchlist entry is too long")
            }
            Self::InvalidWatchlistEntry => {
                formatter.write_str("invasion watchlist entry is invalid")
            }
            Self::ActivityDoneTooLong => formatter.write_str("activity done list is too long"),
            Self::ActivityDoneEntryTooLong => {
                formatter.write_str("activity done entry is too long")
            }
            Self::InvalidActivityDoneEntry => formatter.write_str("activity done entry is invalid"),
        }
    }
}

impl Error for WarframePrefsError {}

#[derive(Clone, Debug, PartialEq)]
pub struct WarframePrefsLoad {
    pub prefs: WarframePrefs,
    pub warning: Option<String>,
}

pub struct WarframePrefsStore {
    path: PathBuf,
}

impl WarframePrefsStore {
    pub fn from_environment() -> Self {
        Self {
            path: warframe_prefs_path(
                env::var_os("XDG_CONFIG_HOME").as_deref(),
                env::var_os("HOME").as_deref(),
            ),
        }
    }

    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> WarframePrefsLoad {
        match read_prefs(&self.path) {
            FileLoad::Loaded(prefs) => WarframePrefsLoad {
                prefs,
                warning: None,
            },
            FileLoad::Rejected(warning) => WarframePrefsLoad {
                prefs: WarframePrefs::default(),
                warning: Some(warning),
            },
            FileLoad::Missing => WarframePrefsLoad {
                prefs: WarframePrefs::default(),
                warning: None,
            },
        }
    }

    pub fn save(&self, prefs: &WarframePrefs) -> io::Result<()> {
        let validated = prefs
            .clone()
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut contents = serde_json::to_vec_pretty(&validated).map_err(|error| {
            let kind = error.io_error_kind().unwrap_or(io::ErrorKind::Other);
            io::Error::new(kind, error)
        })?;
        contents.push(b'\n');
        if contents.len() > WARFRAME_PREFS_MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "warframe prefs are too large",
            ));
        }

        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "user configuration directory unavailable",
                )
            })?;
        fs::create_dir_all(parent)?;

        let mut temporary = NamedTempFile::new_in(parent)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        temporary.write_all(&contents)?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;
        fs::File::open(parent)?.sync_all().map_err(|source| {
            let kind = source.kind();
            io::Error::new(kind, CommittedSettingsSaveError::new(source))
        })?;
        Ok(())
    }
}

enum FileLoad<T> {
    Missing,
    Loaded(T),
    Rejected(String),
}

fn read_prefs(path: &Path) -> FileLoad<WarframePrefs> {
    let contents = match read_private_file(path) {
        FileLoad::Missing => return FileLoad::Missing,
        FileLoad::Loaded(contents) => contents,
        FileLoad::Rejected(warning) => return FileLoad::Rejected(warning),
    };

    match serde_json::from_slice::<WarframePrefs>(&contents)
        .map_err(|error| error.to_string())
        .and_then(|prefs| prefs.validate().map_err(|error| error.to_string()))
    {
        Ok(prefs) => FileLoad::Loaded(prefs),
        Err(error) => FileLoad::Rejected(format!("invalid warframe prefs: {error}")),
    }
}

fn read_private_file(path: &Path) -> FileLoad<Vec<u8>> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_FLAGS)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return FileLoad::Missing,
        Err(error) => {
            return FileLoad::Rejected(format!("refusing unsafe warframe prefs: {error}"));
        }
    };

    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return FileLoad::Rejected(format!("could not inspect opened warframe prefs: {error}"));
        }
    };
    let mode = metadata.permissions().mode() & 0o7777;
    if !metadata.file_type().is_file() || mode != 0o600 {
        return FileLoad::Rejected(
            "refusing to read unsafe warframe prefs: expected a regular 0600 file".to_owned(),
        );
    }

    let mut contents = Vec::new();
    if let Err(error) = Read::by_ref(&mut file)
        .take((WARFRAME_PREFS_MAX_BYTES + 1) as u64)
        .read_to_end(&mut contents)
    {
        return FileLoad::Rejected(format!("could not read warframe prefs: {error}"));
    }
    if contents.len() > WARFRAME_PREFS_MAX_BYTES {
        return FileLoad::Rejected(format!(
            "warframe prefs file is too large (maximum {WARFRAME_PREFS_MAX_BYTES} bytes)"
        ));
    }
    FileLoad::Loaded(contents)
}

pub fn warframe_prefs_path(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
        let path = PathBuf::from(value.filter(|value| !value.is_empty())?);
        path.is_absolute().then_some(path)
    }

    absolute(xdg_config_home)
        .or_else(|| absolute(home).map(|home| home.join(".config")))
        .map(|root| root.join("overcrow/warframe.json"))
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "warframe_prefs_tests.rs"]
mod tests;
