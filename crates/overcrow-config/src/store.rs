use std::{
    env,
    error::Error,
    ffi::OsStr,
    fmt, fs,
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use tempfile::NamedTempFile;

use crate::LifecycleSettings;

const SETTINGS_OPEN_FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK;
pub const SETTINGS_MAX_BYTES: usize = 1024 * 1024;
pub const SETTINGS_DIAGNOSTIC_MAX_BYTES: usize = SETTINGS_MAX_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsLoad {
    pub settings: LifecycleSettings,
    pub warning: Option<String>,
}

pub struct SettingsStore {
    path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsDiagnostic {
    Unavailable,
    Missing,
    Invalid,
    Valid {
        enabled: bool,
        selected_games: usize,
    },
}

#[derive(Debug)]
pub struct CommittedSettingsSaveError {
    source: io::Error,
}

impl CommittedSettingsSaveError {
    pub fn new(source: io::Error) -> Self {
        Self { source }
    }
}

impl fmt::Display for CommittedSettingsSaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "settings were replaced but parent directory sync failed: {}",
            self.source
        )
    }
}

impl Error for CommittedSettingsSaveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

pub fn settings_save_was_committed(error: &io::Error) -> bool {
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<CommittedSettingsSaveError>())
        .is_some()
}

impl SettingsStore {
    pub fn from_environment() -> Self {
        let xdg_config_home = env::var_os("XDG_CONFIG_HOME");
        let home = env::var_os("HOME");
        Self::from_path(settings_path(xdg_config_home.as_deref(), home.as_deref()))
    }

    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> SettingsLoad {
        self.load_with_after_open(|_| Ok(()))
    }

    pub fn diagnose(&self) -> SettingsDiagnostic {
        self.diagnose_with_after_open(|_| Ok(()))
    }

    fn diagnose_with_after_open<F>(&self, after_open: F) -> SettingsDiagnostic
    where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        if self.path.as_os_str().is_empty() {
            return SettingsDiagnostic::Unavailable;
        }
        let mut file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(SETTINGS_OPEN_FLAGS)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return SettingsDiagnostic::Missing;
            }
            Err(_) => return SettingsDiagnostic::Invalid,
        };
        if after_open(&self.path).is_err() {
            return SettingsDiagnostic::Invalid;
        }
        let Ok(metadata) = file.metadata() else {
            return SettingsDiagnostic::Invalid;
        };
        let mode = metadata.permissions().mode() & 0o7777;
        if !metadata.file_type().is_file() || mode != 0o600 {
            return SettingsDiagnostic::Invalid;
        }

        let Ok(contents) = read_bounded(&mut file, SETTINGS_DIAGNOSTIC_MAX_BYTES) else {
            return SettingsDiagnostic::Invalid;
        };
        let Ok(settings) = serde_json::from_slice::<LifecycleSettings>(&contents) else {
            return SettingsDiagnostic::Invalid;
        };
        let Ok(settings) = settings.validate() else {
            return SettingsDiagnostic::Invalid;
        };
        let Some(selected_games) = settings
            .selected_steam_app_ids
            .len()
            .checked_add(settings.manual_games.len())
        else {
            return SettingsDiagnostic::Invalid;
        };
        SettingsDiagnostic::Valid {
            enabled: settings.enabled,
            selected_games,
        }
    }

    fn load_with_after_open<F>(&self, after_open: F) -> SettingsLoad
    where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        let mut file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(SETTINGS_OPEN_FLAGS)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return SettingsLoad {
                    settings: LifecycleSettings::default(),
                    warning: None,
                };
            }
            Err(error) => {
                return disabled_with_warning(format!(
                    "refusing unsafe lifecycle settings: {error}"
                ));
            }
        };

        if let Err(error) = after_open(&self.path) {
            return disabled_with_warning(format!(
                "could not inspect opened lifecycle settings: {error}"
            ));
        }

        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                return disabled_with_warning(format!(
                    "could not inspect opened lifecycle settings: {error}"
                ));
            }
        };
        let private_mode = metadata.permissions().mode() & 0o7777;
        if !metadata.file_type().is_file() || private_mode != 0o600 {
            return disabled_with_warning(
                "refusing to read unsafe lifecycle settings: expected a regular 0600 file"
                    .to_owned(),
            );
        }

        let contents = match read_bounded(&mut file, SETTINGS_MAX_BYTES) {
            Ok(contents) => contents,
            Err(error) => {
                return disabled_with_warning(format!(
                    "could not read lifecycle settings: {error}"
                ));
            }
        };

        match serde_json::from_slice::<LifecycleSettings>(&contents)
            .map_err(|error| error.to_string())
            .and_then(|settings| settings.validate().map_err(|error| error.to_string()))
        {
            Ok(settings) => SettingsLoad {
                settings,
                warning: None,
            },
            Err(error) => disabled_with_warning(format!("invalid lifecycle settings: {error}")),
        }
    }

    pub fn save(&self, settings: &LifecycleSettings) -> io::Result<()> {
        self.save_with_writer(settings, &FileAtomicWriter)
    }

    fn save_with_writer<W>(&self, settings: &LifecycleSettings, writer: &W) -> io::Result<()>
    where
        W: AtomicWriter,
    {
        settings
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
        writer.write_settings(&mut temporary, settings)?;
        temporary.flush()?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        temporary.as_file().sync_all()?;
        writer.persist(temporary, &self.path)?;
        writer.sync_parent(parent).map_err(|source| {
            let kind = source.kind();
            io::Error::new(kind, CommittedSettingsSaveError::new(source))
        })
    }
}

fn read_bounded(file: &mut fs::File, max_bytes: usize) -> io::Result<Vec<u8>> {
    let mut contents = Vec::new();
    Read::by_ref(file)
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut contents)?;
    if contents.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("settings file is too large (maximum {max_bytes} bytes)"),
        ));
    }
    Ok(contents)
}

fn disabled_with_warning(warning: String) -> SettingsLoad {
    SettingsLoad {
        settings: LifecycleSettings::default(),
        warning: Some(warning),
    }
}

fn settings_path(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
        let path = PathBuf::from(value.filter(|value| !value.is_empty())?);
        path.is_absolute().then_some(path)
    }

    absolute(xdg_config_home)
        .or_else(|| absolute(home).map(|home| home.join(".config")))
        .map(|root| root.join("overcrow/settings.json"))
        .unwrap_or_default()
}

trait AtomicWriter {
    fn write_settings(
        &self,
        temporary: &mut NamedTempFile,
        settings: &LifecycleSettings,
    ) -> io::Result<()>;

    fn persist(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()>;

    fn sync_parent(&self, parent: &Path) -> io::Result<()>;
}

struct FileAtomicWriter;

impl AtomicWriter for FileAtomicWriter {
    fn write_settings(
        &self,
        temporary: &mut NamedTempFile,
        settings: &LifecycleSettings,
    ) -> io::Result<()> {
        write_settings_json(temporary, settings)
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

fn write_settings_json(writer: &mut impl Write, settings: &LifecycleSettings) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *writer, settings).map_err(|error| {
        let kind = error.io_error_kind().unwrap_or(io::ErrorKind::Other);
        io::Error::new(kind, error)
    })?;
    writer.write_all(b"\n")
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;
