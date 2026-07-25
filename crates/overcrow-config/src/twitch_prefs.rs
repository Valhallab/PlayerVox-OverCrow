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

pub const TWITCH_PREFS_SCHEMA_VERSION: u32 = 1;
pub const TWITCH_PREFS_MAX_BYTES: usize = 16 * 1024;
pub const TWITCH_CHANNEL_MAX_CHARS: usize = 25;
pub const TWITCH_FAVORITES_MAX: usize = 20;
pub const TWITCH_PASSIVE_LIFETIME_MIN_SECS: u32 = 5;
pub const TWITCH_PASSIVE_LIFETIME_MAX_SECS: u32 = 120;
pub const TWITCH_PASSIVE_LIFETIME_DEFAULT_SECS: u32 = 30;

const OPEN_FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TwitchPrefs {
    pub schema_version: u32,
    pub active_channel: Option<String>,
    pub favorites: Vec<String>,
    pub passive_lifetime_secs: u32,
}

impl Default for TwitchPrefs {
    fn default() -> Self {
        Self {
            schema_version: TWITCH_PREFS_SCHEMA_VERSION,
            active_channel: None,
            favorites: Vec::new(),
            passive_lifetime_secs: TWITCH_PASSIVE_LIFETIME_DEFAULT_SECS,
        }
    }
}

impl TwitchPrefs {
    pub fn validate(self) -> Result<Self, TwitchPrefsError> {
        if self.schema_version != TWITCH_PREFS_SCHEMA_VERSION {
            return Err(TwitchPrefsError::UnsupportedSchemaVersion);
        }
        if !(TWITCH_PASSIVE_LIFETIME_MIN_SECS..=TWITCH_PASSIVE_LIFETIME_MAX_SECS)
            .contains(&self.passive_lifetime_secs)
        {
            return Err(TwitchPrefsError::InvalidPassiveLifetime);
        }

        let active_channel = self
            .active_channel
            .as_deref()
            .map(normalize_twitch_channel)
            .transpose()?;
        let mut favorites = Vec::with_capacity(self.favorites.len().min(TWITCH_FAVORITES_MAX));
        for favorite in self.favorites {
            let favorite = normalize_twitch_channel(&favorite)?;
            if !favorites.contains(&favorite) {
                if favorites.len() == TWITCH_FAVORITES_MAX {
                    return Err(TwitchPrefsError::TooManyFavorites);
                }
                favorites.push(favorite);
            }
        }

        Ok(Self {
            schema_version: self.schema_version,
            active_channel,
            favorites,
            passive_lifetime_secs: self.passive_lifetime_secs,
        })
    }
}

pub fn normalize_twitch_channel(value: &str) -> Result<String, TwitchPrefsError> {
    let trimmed = value.trim();
    let login = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if login.is_empty()
        || login.len() > TWITCH_CHANNEL_MAX_CHARS
        || !login
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(TwitchPrefsError::InvalidChannel);
    }
    Ok(login.to_ascii_lowercase())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TwitchPrefsError {
    UnsupportedSchemaVersion,
    InvalidChannel,
    TooManyFavorites,
    InvalidPassiveLifetime,
}

impl fmt::Display for TwitchPrefsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion => {
                formatter.write_str("unsupported Twitch preferences schema version")
            }
            Self::InvalidChannel => formatter.write_str("invalid Twitch channel"),
            Self::TooManyFavorites => formatter.write_str("too many favorite Twitch channels"),
            Self::InvalidPassiveLifetime => {
                formatter.write_str("invalid passive Twitch message lifetime")
            }
        }
    }
}

impl Error for TwitchPrefsError {}

#[derive(Clone, Debug, PartialEq)]
pub struct TwitchPrefsLoad {
    pub prefs: TwitchPrefs,
    pub warning: Option<String>,
}

pub struct TwitchPrefsStore {
    path: PathBuf,
}

impl TwitchPrefsStore {
    pub fn from_environment() -> Self {
        Self {
            path: twitch_prefs_path(
                env::var_os("XDG_CONFIG_HOME").as_deref(),
                env::var_os("HOME").as_deref(),
            ),
        }
    }

    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> TwitchPrefsLoad {
        match read_prefs(&self.path) {
            FileLoad::Loaded(prefs) => TwitchPrefsLoad {
                prefs,
                warning: None,
            },
            FileLoad::Rejected(warning) => TwitchPrefsLoad {
                prefs: TwitchPrefs::default(),
                warning: Some(warning),
            },
            FileLoad::Missing => TwitchPrefsLoad {
                prefs: TwitchPrefs::default(),
                warning: None,
            },
        }
    }

    pub fn save(&self, prefs: &TwitchPrefs) -> io::Result<()> {
        let validated = prefs
            .clone()
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut contents = serde_json::to_vec_pretty(&validated).map_err(|error| {
            let kind = error.io_error_kind().unwrap_or(io::ErrorKind::Other);
            io::Error::new(kind, error)
        })?;
        contents.push(b'\n');
        if contents.len() > TWITCH_PREFS_MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Twitch preferences are too large",
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

fn read_prefs(path: &Path) -> FileLoad<TwitchPrefs> {
    let contents = match read_private_file(path) {
        FileLoad::Missing => return FileLoad::Missing,
        FileLoad::Loaded(contents) => contents,
        FileLoad::Rejected(warning) => return FileLoad::Rejected(warning),
    };

    match serde_json::from_slice::<TwitchPrefs>(&contents)
        .map_err(|error| error.to_string())
        .and_then(|prefs| prefs.validate().map_err(|error| error.to_string()))
    {
        Ok(prefs) => FileLoad::Loaded(prefs),
        Err(error) => FileLoad::Rejected(format!("invalid Twitch preferences: {error}")),
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
            return FileLoad::Rejected(format!("refusing unsafe Twitch preferences: {error}"));
        }
    };

    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return FileLoad::Rejected(format!(
                "could not inspect opened Twitch preferences: {error}"
            ));
        }
    };
    let mode = metadata.permissions().mode() & 0o7777;
    if !metadata.file_type().is_file() || mode != 0o600 {
        return FileLoad::Rejected(
            "refusing to read unsafe Twitch preferences: expected a regular 0600 file".to_owned(),
        );
    }

    let mut contents = Vec::new();
    if let Err(error) = Read::by_ref(&mut file)
        .take((TWITCH_PREFS_MAX_BYTES + 1) as u64)
        .read_to_end(&mut contents)
    {
        return FileLoad::Rejected(format!("could not read Twitch preferences: {error}"));
    }
    if contents.len() > TWITCH_PREFS_MAX_BYTES {
        return FileLoad::Rejected(format!(
            "Twitch preferences file is too large (maximum {TWITCH_PREFS_MAX_BYTES} bytes)"
        ));
    }
    FileLoad::Loaded(contents)
}

pub fn twitch_prefs_path(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
        let path = PathBuf::from(value.filter(|value| !value.is_empty())?);
        path.is_absolute().then_some(path)
    }

    absolute(xdg_config_home)
        .or_else(|| absolute(home).map(|home| home.join(".config")))
        .map(|root| root.join("overcrow/twitch.json"))
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "twitch_prefs_tests.rs"]
mod tests;
