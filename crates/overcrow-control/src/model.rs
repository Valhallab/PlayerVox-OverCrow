use std::{
    collections::BTreeSet,
    error::Error,
    fmt, fs, io,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};

use overcrow_config::{LifecycleSettings, ManualGame, SettingsLoad, SettingsStore};
use sha2::{Digest, Sha256};
use unicode_casefold::UnicodeCaseFold;

use crate::{DiscoveryReport, SteamGame};

const MANUAL_ID_PREFIX: &str = "local.";
const MANUAL_ID_DIGEST_BYTES: usize = 16;

/// Validates a user-selected executable and returns its canonical identity path.
pub trait PathValidator: Send + Sync {
    fn canonical_executable(&self, path: &Path) -> Result<PathBuf, SelectionError>;
}

pub trait SelectionStore: Send {
    fn save(&self, settings: &LifecycleSettings) -> io::Result<()>;
}

impl SelectionStore for SettingsStore {
    fn save(&self, settings: &LifecycleSettings) -> io::Result<()> {
        SettingsStore::save(self, settings)
    }
}

/// Filesystem-backed validator for native Linux executables.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativePathValidator;

impl PathValidator for NativePathValidator {
    fn canonical_executable(&self, path: &Path) -> Result<PathBuf, SelectionError> {
        if !path.is_absolute() {
            return Err(SelectionError::ExecutableNotAbsolute);
        }
        if has_exe_extension(path) {
            return Err(SelectionError::WineIdentityUnavailable);
        }

        let canonical = fs::canonicalize(path)
            .map_err(|error| SelectionError::ExecutableValidationFailed(error.kind()))?;
        if has_exe_extension(&canonical) {
            return Err(SelectionError::WineIdentityUnavailable);
        }

        // Inspect the canonical path without following a last-component symlink that could
        // have been swapped in after canonicalization. Returning a path rather than an open
        // descriptor cannot eliminate later replacement, so runtime identity must revalidate.
        let metadata = fs::symlink_metadata(&canonical)
            .map_err(|error| SelectionError::ExecutableValidationFailed(error.kind()))?;
        if !metadata.file_type().is_file() {
            return Err(SelectionError::ExecutableNotRegularFile);
        }
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(SelectionError::ExecutableNotExecutable);
        }

        Ok(canonical)
    }
}

pub struct ControlModel {
    pub settings: LifecycleSettings,
    pub games: Vec<SteamGame>,
    pub discovery_warnings: Vec<String>,
    pub settings_warning: Option<String>,
    search: String,
    path_validator: Box<dyn PathValidator>,
}

impl ControlModel {
    pub fn new<V>(
        settings_load: SettingsLoad,
        discovery: DiscoveryReport,
        path_validator: V,
    ) -> Self
    where
        V: PathValidator + 'static,
    {
        let SettingsLoad {
            mut settings,
            warning,
        } = settings_load;
        let (manual_games, dropped_manual_games) =
            revalidated_loaded_manual_games(&settings.manual_games, &path_validator);
        settings.manual_games = manual_games;
        if dropped_manual_games != 0 {
            settings.enabled = false;
        }

        Self {
            settings,
            games: discovery.games,
            discovery_warnings: discovery.warnings,
            settings_warning: merged_settings_warning(warning, dropped_manual_games),
            search: String::new(),
            path_validator: Box::new(path_validator),
        }
    }

    pub fn set_search(&mut self, search: &str) {
        self.search = search.trim().case_fold().collect();
    }

    pub(crate) fn apply_settings_load(&mut self, settings_load: SettingsLoad) {
        let SettingsLoad {
            mut settings,
            warning,
        } = settings_load;
        let (manual_games, dropped_manual_games) =
            revalidated_loaded_manual_games(&settings.manual_games, self.path_validator.as_ref());
        settings.manual_games = manual_games;
        if dropped_manual_games != 0 {
            settings.enabled = false;
        }
        self.settings = settings;
        self.settings_warning = merged_settings_warning(warning, dropped_manual_games);
    }

    pub fn visible_games(&self) -> Vec<&SteamGame> {
        self.games
            .iter()
            .filter(|game| {
                self.search.is_empty()
                    || game
                        .name
                        .as_str()
                        .case_fold()
                        .collect::<String>()
                        .contains(&self.search)
            })
            .collect()
    }

    pub fn set_steam_selected(&mut self, app_id: u32, selected: bool) {
        if selected {
            if self.games.iter().any(|game| game.app_id == app_id) {
                self.settings.selected_steam_app_ids.insert(app_id);
            }
        } else {
            self.settings.selected_steam_app_ids.remove(&app_id);
        }
    }

    pub fn add_manual_game(
        &mut self,
        name: &str,
        executable: &Path,
    ) -> Result<String, SelectionError> {
        let game = validated_manual_game(self.path_validator.as_ref(), name, executable)?;
        if self
            .settings
            .manual_games
            .iter()
            .any(|existing| existing.executable == game.executable)
        {
            return Err(SelectionError::DuplicateManualExecutable);
        }
        if self
            .settings
            .manual_games
            .iter()
            .any(|existing| existing.id == game.id)
        {
            return Err(SelectionError::ManualGameIdCollision);
        }

        let id = game.id.clone();
        self.settings.manual_games.push(game);
        Ok(id)
    }

    pub fn remove_manual_game(&mut self, id: &str) -> bool {
        let original_len = self.settings.manual_games.len();
        self.settings.manual_games.retain(|game| game.id != id);
        self.settings.manual_games.len() != original_len
    }

    pub fn save_selections(&self, store: &dyn SelectionStore) -> io::Result<()> {
        let mut settings = self.settings.clone();
        let (validated_manual_games, dropped_manual_games) =
            revalidated_loaded_manual_games(&settings.manual_games, self.path_validator.as_ref());
        if dropped_manual_games != 0 || validated_manual_games != settings.manual_games {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "manual game selections changed or failed revalidation",
            ));
        }
        settings.manual_games = validated_manual_games;
        store.save(&settings)
    }
}

fn validated_manual_game(
    path_validator: &dyn PathValidator,
    name: &str,
    executable: &Path,
) -> Result<ManualGame, SelectionError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(SelectionError::EmptyName);
    }
    if !executable.is_absolute() {
        return Err(SelectionError::ExecutableNotAbsolute);
    }

    let canonical = path_validator.canonical_executable(executable)?;
    if !canonical.is_absolute() {
        return Err(SelectionError::ExecutableNotAbsolute);
    }
    if canonical.to_str().is_none() {
        return Err(SelectionError::ExecutableNotUtf8);
    }
    if has_exe_extension(&canonical) {
        return Err(SelectionError::WineIdentityUnavailable);
    }

    Ok(ManualGame {
        id: stable_manual_game_id(&canonical),
        name: name.to_owned(),
        executable: canonical,
    })
}

pub(crate) fn validate_stored_manual_game(
    path_validator: &dyn PathValidator,
    game: &ManualGame,
) -> Result<(), String> {
    // Runtime manual identity is pathname-based: the current canonical path, execute/file
    // metadata, stable path-derived ID, and normalized name must still match exactly. Content or
    // inode replacement at the same canonical pathname is intentionally not a distinct identity.
    let validated = validated_manual_game(
        path_validator,
        game.name.as_str(),
        game.executable.as_path(),
    )
    .map_err(|error| error.to_string())?;
    if validated != *game {
        return Err("stored manual game identity changed after validation".to_owned());
    }
    Ok(())
}

fn revalidated_loaded_manual_games(
    manual_games: &[ManualGame],
    path_validator: &dyn PathValidator,
) -> (Vec<ManualGame>, usize) {
    let mut retained = Vec::with_capacity(manual_games.len());
    let mut canonical_paths = BTreeSet::new();
    let mut stable_ids = BTreeSet::new();
    let mut dropped = 0;

    for loaded in manual_games {
        let validated = validated_manual_game(
            path_validator,
            loaded.name.as_str(),
            loaded.executable.as_path(),
        );
        let Ok(validated) = validated else {
            dropped += 1;
            continue;
        };

        let identity_is_unchanged = loaded.executable == validated.executable
            && loaded.id == validated.id
            && !canonical_paths.contains(&validated.executable)
            && !stable_ids.contains(&validated.id);
        if !identity_is_unchanged {
            dropped += 1;
            continue;
        }

        canonical_paths.insert(validated.executable.clone());
        stable_ids.insert(validated.id.clone());
        retained.push(validated);
    }

    (retained, dropped)
}

fn merged_settings_warning(
    existing: Option<String>,
    dropped_manual_games: usize,
) -> Option<String> {
    if dropped_manual_games == 0 {
        return existing;
    }

    let plural = if dropped_manual_games == 1 { "" } else { "s" };
    let dropped = format!("Dropped {dropped_manual_games} invalid manual game selection{plural}.");
    Some(match existing {
        Some(existing) => format!("{existing} {dropped}"),
        None => dropped,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionError {
    EmptyName,
    ExecutableNotAbsolute,
    ExecutableNotUtf8,
    ExecutableValidationFailed(io::ErrorKind),
    ExecutableNotRegularFile,
    ExecutableNotExecutable,
    WineIdentityUnavailable,
    DuplicateManualExecutable,
    ManualGameIdCollision,
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("manual game name must not be empty"),
            Self::ExecutableNotAbsolute => {
                formatter.write_str("manual game executable must be absolute")
            }
            Self::ExecutableNotUtf8 => {
                formatter.write_str("manual game executable must be valid UTF-8")
            }
            Self::ExecutableValidationFailed(kind) => {
                write!(
                    formatter,
                    "could not validate manual game executable: {kind}"
                )
            }
            Self::ExecutableNotRegularFile => {
                formatter.write_str("manual game executable must be a regular file")
            }
            Self::ExecutableNotExecutable => formatter
                .write_str("manual game executable must have at least one Unix execute bit"),
            Self::WineIdentityUnavailable => {
                formatter.write_str("exact Wine executable identity is not yet available")
            }
            Self::DuplicateManualExecutable => {
                formatter.write_str("manual game executable is already selected")
            }
            Self::ManualGameIdCollision => {
                formatter.write_str("manual game stable ID is already used by another executable")
            }
        }
    }
}

impl Error for SelectionError {}

fn has_exe_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
}

/// Hashes the canonical Unix pathname's raw `OsStr` bytes, without locale or lossy UTF-8
/// conversion, and uses the first 128 digest bits as a fixed, lowercase hexadecimal ID suffix.
pub(crate) fn stable_manual_game_id(canonical: &Path) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let digest = Sha256::digest(canonical.as_os_str().as_bytes());
    let mut id = String::with_capacity(MANUAL_ID_PREFIX.len() + MANUAL_ID_DIGEST_BYTES * 2);
    id.push_str(MANUAL_ID_PREFIX);
    for byte in &digest[..MANUAL_ID_DIGEST_BYTES] {
        id.push(HEX[usize::from(byte >> 4)] as char);
        id.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    id
}

pub(crate) fn is_stable_manual_game_id(id: &str) -> bool {
    id.strip_prefix(MANUAL_ID_PREFIX).is_some_and(|digest| {
        digest.len() == MANUAL_ID_DIGEST_BYTES * 2
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
