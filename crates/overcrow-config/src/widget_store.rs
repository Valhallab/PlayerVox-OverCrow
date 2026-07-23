use std::{
    env,
    ffi::OsStr,
    fs,
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::Deserialize;
use tempfile::NamedTempFile;

use crate::{CommittedSettingsSaveError, WidgetPosition, WidgetProfile};

const WIDGET_OPEN_FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK;
const WIDGET_MAX_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetSettingsLoad {
    pub profile: WidgetProfile,
    pub warning: Option<String>,
}

pub struct WidgetSettingsStore {
    path: PathBuf,
    legacy_path: PathBuf,
}

impl WidgetSettingsStore {
    pub fn from_environment() -> Self {
        let (path, legacy_path) = widget_paths(
            env::var_os("XDG_CONFIG_HOME").as_deref(),
            env::var_os("HOME").as_deref(),
        );
        Self { path, legacy_path }
    }

    pub fn from_paths(path: impl Into<PathBuf>, legacy_path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            legacy_path: legacy_path.into(),
        }
    }

    pub fn load(&self) -> WidgetSettingsLoad {
        match read_profile(&self.path) {
            FileLoad::Loaded(profile) => WidgetSettingsLoad {
                profile,
                warning: None,
            },
            FileLoad::Rejected(warning) => defaults_with_warning(warning),
            FileLoad::Missing => match read_legacy_preferences(&self.legacy_path) {
                FileLoad::Loaded(legacy) => {
                    let mut profile = WidgetProfile::default();
                    profile.session.show_in_passive = legacy.show_stopwatch_in_passive;
                    profile.session.position = legacy.stopwatch_position;
                    WidgetSettingsLoad {
                        profile,
                        warning: None,
                    }
                }
                FileLoad::Rejected(warning) => defaults_with_warning(warning),
                FileLoad::Missing => WidgetSettingsLoad {
                    profile: WidgetProfile::default(),
                    warning: None,
                },
            },
        }
    }

    pub fn save(&self, profile: &WidgetProfile) -> io::Result<()> {
        self.save_with_writer(profile, &FileAtomicWriter)
    }

    fn save_with_writer<W>(&self, profile: &WidgetProfile, writer: &W) -> io::Result<()>
    where
        W: AtomicWriter,
    {
        profile
            .clone()
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

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
        writer.write_profile(&mut temporary, profile)?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        writer.persist(temporary, &self.path)?;
        writer.sync_parent(parent).map_err(|source| {
            let kind = source.kind();
            io::Error::new(kind, CommittedSettingsSaveError::new(source))
        })
    }
}

enum FileLoad<T> {
    Missing,
    Loaded(T),
    Rejected(String),
}

fn read_profile(path: &Path) -> FileLoad<WidgetProfile> {
    let contents = match read_private_file(path, "widget settings") {
        FileLoad::Missing => return FileLoad::Missing,
        FileLoad::Loaded(contents) => contents,
        FileLoad::Rejected(warning) => return FileLoad::Rejected(warning),
    };

    match serde_json::from_slice::<WidgetProfile>(&contents)
        .map_err(|error| error.to_string())
        .and_then(|profile| profile.validate().map_err(|error| error.to_string()))
    {
        Ok(profile) => FileLoad::Loaded(profile),
        Err(error) => FileLoad::Rejected(format!("invalid widget settings: {error}")),
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyOverlayPreferences {
    show_stopwatch_in_passive: bool,
    stopwatch_position: WidgetPosition,
}

fn read_legacy_preferences(path: &Path) -> FileLoad<LegacyOverlayPreferences> {
    let contents = match read_private_file(path, "legacy widget preferences") {
        FileLoad::Missing => return FileLoad::Missing,
        FileLoad::Loaded(contents) => contents,
        FileLoad::Rejected(warning) => return FileLoad::Rejected(warning),
    };

    match serde_json::from_slice::<LegacyOverlayPreferences>(&contents) {
        Ok(legacy) if legacy.stopwatch_position.is_valid() => FileLoad::Loaded(legacy),
        Ok(_) => FileLoad::Rejected("invalid legacy widget preferences: position".to_owned()),
        Err(error) => FileLoad::Rejected(format!("invalid legacy widget preferences: {error}")),
    }
}

fn read_private_file(path: &Path, description: &str) -> FileLoad<Vec<u8>> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(WIDGET_OPEN_FLAGS)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return FileLoad::Missing,
        Err(error) => {
            return FileLoad::Rejected(format!("refusing unsafe {description}: {error}"));
        }
    };

    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return FileLoad::Rejected(format!("could not inspect opened {description}: {error}"));
        }
    };
    let mode = metadata.permissions().mode() & 0o7777;
    if !metadata.file_type().is_file() || mode != 0o600 {
        return FileLoad::Rejected(format!(
            "refusing to read unsafe {description}: expected a regular 0600 file"
        ));
    }

    let mut contents = Vec::new();
    if let Err(error) = Read::by_ref(&mut file)
        .take((WIDGET_MAX_BYTES + 1) as u64)
        .read_to_end(&mut contents)
    {
        return FileLoad::Rejected(format!("could not read {description}: {error}"));
    }
    if contents.len() > WIDGET_MAX_BYTES {
        return FileLoad::Rejected(format!(
            "{description} file is too large (maximum {WIDGET_MAX_BYTES} bytes)"
        ));
    }
    FileLoad::Loaded(contents)
}

fn defaults_with_warning(warning: String) -> WidgetSettingsLoad {
    WidgetSettingsLoad {
        profile: WidgetProfile::default(),
        warning: Some(warning),
    }
}

fn widget_paths(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> (PathBuf, PathBuf) {
    fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
        let path = PathBuf::from(value.filter(|value| !value.is_empty())?);
        path.is_absolute().then_some(path)
    }

    absolute(xdg_config_home)
        .or_else(|| absolute(home).map(|home| home.join(".config")))
        .map(|root| {
            let directory = root.join("overcrow");
            (
                directory.join("widgets.json"),
                directory.join("overlay.json"),
            )
        })
        .unwrap_or_default()
}

trait AtomicWriter {
    fn write_profile(
        &self,
        temporary: &mut NamedTempFile,
        profile: &WidgetProfile,
    ) -> io::Result<()>;

    fn persist(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()>;

    fn sync_parent(&self, parent: &Path) -> io::Result<()>;
}

struct FileAtomicWriter;

impl AtomicWriter for FileAtomicWriter {
    fn write_profile(
        &self,
        temporary: &mut NamedTempFile,
        profile: &WidgetProfile,
    ) -> io::Result<()> {
        serde_json::to_writer_pretty(&mut *temporary, profile).map_err(|error| {
            let kind = error.io_error_kind().unwrap_or(io::ErrorKind::Other);
            io::Error::new(kind, error)
        })?;
        temporary.write_all(b"\n")
    }

    fn persist(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()> {
        temporary
            .persist(destination)
            .map(|_| ())
            .map_err(|error| error.error)
    }

    fn sync_parent(&self, parent: &Path) -> io::Result<()> {
        fs::File::open(parent)?.sync_all()
    }
}

#[cfg(test)]
#[path = "widget_store_tests.rs"]
mod tests;
