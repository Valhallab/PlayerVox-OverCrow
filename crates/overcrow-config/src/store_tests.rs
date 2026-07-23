use std::{
    cell::RefCell,
    ffi::OsStr,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::{ffi::CString, os::unix::ffi::OsStrExt};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

use tempfile::NamedTempFile;

use super::{
    AtomicWriter, FileAtomicWriter, SETTINGS_DIAGNOSTIC_MAX_BYTES, SETTINGS_MAX_BYTES,
    SETTINGS_OPEN_FLAGS, SettingsDiagnostic, SettingsStore, settings_path,
    settings_save_was_committed, write_settings_json,
};
use crate::{LifecycleSettings, ManualGame, SettingsError};

#[test]
fn missing_settings_are_safely_disabled() {
    let temp = tempfile::tempdir().unwrap();

    let result = SettingsStore::from_path(temp.path().join("settings.json")).load();

    assert_eq!(result.settings, LifecycleSettings::default());
    assert!(result.warning.is_none());
}

#[test]
fn malformed_json_is_disabled_with_a_warning() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    write_private(&path, b"not json");

    let result = SettingsStore::from_path(path).load();

    assert_eq!(result.settings, LifecycleSettings::default());
    assert!(result.warning.unwrap().contains("invalid"));
}

#[test]
fn oversized_settings_are_disabled_with_a_warning() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    write_private(&path, &vec![b' '; SETTINGS_MAX_BYTES + 1]);

    let result = SettingsStore::from_path(path).load();

    assert_eq!(result.settings, LifecycleSettings::default());
    assert!(result.warning.unwrap().contains("too large"));
}

#[test]
fn invalid_model_content_is_disabled_with_a_warning() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    write_private(
        &path,
        br#"{
            "schema_version": 2,
            "enabled": true,
            "selected_steam_app_ids": [620],
            "manual_games": [],
            "shortcut": {"enabled": true, "accelerator": "Meta+Alt+O"}
        }"#,
    );

    let result = SettingsStore::from_path(path).load();

    assert_eq!(result.settings, LifecycleSettings::default());
    assert!(result.warning.unwrap().contains("invalid"));
}

#[test]
fn read_failures_are_disabled_with_a_warning() {
    let temp = tempfile::tempdir().unwrap();

    let result = SettingsStore::from_path(temp.path()).load();

    assert_eq!(result.settings, LifecycleSettings::default());
    assert!(result.warning.unwrap().contains("read"));
}

#[cfg(unix)]
#[test]
fn symlinked_valid_enabled_settings_are_unsafe_and_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("enabled-target.json");
    let path = temp.path().join("settings.json");
    write_private(&target, &enabled_settings_json());
    symlink(&target, &path).unwrap();

    let result = SettingsStore::from_path(path).load();

    assert_eq!(result.settings, LifecycleSettings::default());
    assert!(result.warning.unwrap().contains("unsafe"));
}

#[cfg(unix)]
#[test]
fn settings_open_flags_include_nonblocking_fifo_protection() {
    assert_ne!(SETTINGS_OPEN_FLAGS & libc::O_NONBLOCK, 0);
}

#[cfg(unix)]
#[test]
fn fifo_settings_are_rejected_without_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    make_fifo(&path);

    let result = SettingsStore::from_path(path).load();

    assert_eq!(result.settings, LifecycleSettings::default());
    assert!(result.warning.unwrap().contains("unsafe"));
}

#[cfg(unix)]
#[test]
fn non_0600_valid_enabled_settings_are_unsafe_and_disabled() {
    let temp = tempfile::tempdir().unwrap();

    for mode in [0o400, 0o640, 0o604, 0o700, 0o4600] {
        let path = temp.path().join(format!("settings-{mode:o}.json"));
        fs::write(&path, enabled_settings_json()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();

        let result = SettingsStore::from_path(path).load();

        assert_eq!(
            result.settings,
            LifecycleSettings::default(),
            "mode {mode:o} must not gain lifecycle authority"
        );
        assert!(result.warning.unwrap().contains("unsafe"));
    }
}

#[cfg(unix)]
#[test]
fn load_uses_the_opened_handle_when_the_path_is_swapped() {
    for swap in [PathSwap::Symlink, PathSwap::RegularFile] {
        assert_load_uses_the_opened_handle(swap);
    }
}

#[cfg(unix)]
#[test]
fn diagnostic_uses_one_opened_handle_when_the_path_is_swapped() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let replacement_path = temp.path().join("replacement.json");
    let mut original = LifecycleSettings::default();
    original.selected_steam_app_ids.insert(620);
    write_private(&path, &serde_json::to_vec(&original).unwrap());
    write_private(&replacement_path, &enabled_settings_json());

    let diagnostic = SettingsStore::from_path(&path)
        .diagnose_with_after_open(|opened_path| fs::rename(&replacement_path, opened_path));

    assert_eq!(
        diagnostic,
        SettingsDiagnostic::Valid {
            enabled: false,
            selected_games: 1,
        }
    );
}

#[cfg(unix)]
#[test]
fn diagnostic_rejects_symlinks_nonregular_modes_schema_and_oversize() {
    let temp = tempfile::tempdir().unwrap();
    let valid_target = temp.path().join("valid-target.json");
    write_private(&valid_target, &enabled_settings_json());

    let symlink_path = temp.path().join("symlink.json");
    symlink(&valid_target, &symlink_path).unwrap();
    assert_eq!(
        SettingsStore::from_path(symlink_path).diagnose(),
        SettingsDiagnostic::Invalid
    );

    assert_eq!(
        SettingsStore::from_path(temp.path()).diagnose(),
        SettingsDiagnostic::Invalid
    );

    let public_path = temp.path().join("public.json");
    fs::write(&public_path, enabled_settings_json()).unwrap();
    fs::set_permissions(&public_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        SettingsStore::from_path(public_path).diagnose(),
        SettingsDiagnostic::Invalid
    );

    let invalid_path = temp.path().join("invalid.json");
    write_private(&invalid_path, b"{}");
    assert_eq!(
        SettingsStore::from_path(invalid_path).diagnose(),
        SettingsDiagnostic::Invalid
    );

    let oversized_path = temp.path().join("oversized.json");
    write_private(
        &oversized_path,
        &vec![b' '; SETTINGS_DIAGNOSTIC_MAX_BYTES + 1],
    );
    assert_eq!(
        SettingsStore::from_path(oversized_path).diagnose(),
        SettingsDiagnostic::Invalid
    );
}

#[test]
fn diagnostic_distinguishes_missing_unavailable_and_valid_settings() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        SettingsStore::from_path(temp.path().join("missing.json")).diagnose(),
        SettingsDiagnostic::Missing
    );
    assert_eq!(
        SettingsStore::from_path(PathBuf::new()).diagnose(),
        SettingsDiagnostic::Unavailable
    );

    let path = temp.path().join("valid.json");
    let mut settings = LifecycleSettings {
        enabled: true,
        ..LifecycleSettings::default()
    };
    settings.selected_steam_app_ids.extend([620, 1_623_730]);
    settings.manual_games.push(ManualGame {
        id: "local.portal".into(),
        name: "Portal".into(),
        executable: PathBuf::from("/games/portal"),
    });
    write_private(&path, &serde_json::to_vec(&settings).unwrap());
    assert_eq!(
        SettingsStore::from_path(path).diagnose(),
        SettingsDiagnostic::Valid {
            enabled: true,
            selected_games: 3,
        }
    );
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
enum PathSwap {
    Symlink,
    RegularFile,
}

#[cfg(unix)]
fn assert_load_uses_the_opened_handle(swap: PathSwap) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let replacement_path = temp.path().join("enabled-replacement.json");
    let mut original = LifecycleSettings::default();
    original.selected_steam_app_ids.insert(620);
    write_private(&path, &serde_json::to_vec(&original).unwrap());
    write_private(&replacement_path, &enabled_settings_json());
    let store = SettingsStore::from_path(&path);

    let result = store.load_with_after_open(|opened_path| {
        match swap {
            PathSwap::Symlink => {
                fs::remove_file(opened_path)?;
                symlink(&replacement_path, opened_path)?;
            }
            PathSwap::RegularFile => fs::rename(&replacement_path, opened_path)?,
        }
        Ok(())
    });

    assert_eq!(
        result.settings, original,
        "the {swap:?} swap must not change the opened settings"
    );
    assert!(result.warning.is_none());
}

#[test]
fn xdg_config_home_takes_precedence_over_home() {
    assert_eq!(
        settings_path(Some(OsStr::new("/xdg")), Some(OsStr::new("/home/player"))),
        PathBuf::from("/xdg/overcrow/settings.json")
    );
}

#[test]
fn home_is_used_when_xdg_config_home_is_missing_or_empty() {
    for xdg_config_home in [None, Some(OsStr::new(""))] {
        assert_eq!(
            settings_path(xdg_config_home, Some(OsStr::new("/home/player"))),
            PathBuf::from("/home/player/.config/overcrow/settings.json")
        );
    }
}

#[test]
fn relative_xdg_config_home_falls_back_to_absolute_home() {
    assert_eq!(
        settings_path(
            Some(OsStr::new("relative-xdg")),
            Some(OsStr::new("/home/player"))
        ),
        PathBuf::from("/home/player/.config/overcrow/settings.json")
    );
}

#[test]
fn relative_home_does_not_produce_an_authority_path() {
    assert_eq!(
        settings_path(None, Some(OsStr::new("relative-home"))),
        PathBuf::new()
    );
    assert_eq!(
        settings_path(
            Some(OsStr::new("relative-xdg")),
            Some(OsStr::new("relative-home"))
        ),
        PathBuf::new()
    );
}

#[test]
fn missing_environment_roots_produce_an_unavailable_path() {
    assert_eq!(settings_path(None, None), PathBuf::new());
    assert_eq!(
        settings_path(Some(OsStr::new("")), Some(OsStr::new(""))),
        PathBuf::new()
    );
}

#[test]
fn save_creates_parent_and_writes_pretty_json_with_a_newline() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested/overcrow/settings.json");

    SettingsStore::from_path(&path)
        .save(&LifecycleSettings::default())
        .unwrap();

    let bytes = fs::read(path).unwrap();
    assert!(bytes.ends_with(b"\n"));
    assert!(bytes.windows(4).any(|window| window == b"\n  \""));
}

#[cfg(unix)]
#[test]
fn saved_settings_have_private_permissions() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");

    SettingsStore::from_path(&path)
        .save(&LifecycleSettings::default())
        .unwrap();

    let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn save_atomically_replaces_an_existing_valid_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::from_path(&path);
    let mut original = LifecycleSettings::default();
    original.selected_steam_app_ids.insert(620);
    store.save(&original).unwrap();
    let original_bytes = fs::read(&path).unwrap();

    let mut replacement = LifecycleSettings {
        enabled: true,
        ..LifecycleSettings::default()
    };
    replacement.selected_steam_app_ids.insert(1623730);
    store.save(&replacement).unwrap();

    assert_ne!(fs::read(&path).unwrap(), original_bytes);
    assert_eq!(store.load().settings, replacement);
}

#[test]
fn invalid_settings_are_not_saved_over_a_valid_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::from_path(&path);
    store.save(&LifecycleSettings::default()).unwrap();
    let original_bytes = fs::read(&path).unwrap();
    let mut invalid = LifecycleSettings::default();
    invalid.selected_steam_app_ids.insert(0);

    let error = store.save(&invalid).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read(path).unwrap(), original_bytes);
}

#[derive(Clone, Copy)]
enum FailurePoint {
    Serialization,
    Persist,
    SyncParent,
}

struct FailingAtomicWriter(FailurePoint);

impl AtomicWriter for FailingAtomicWriter {
    fn write_settings(
        &self,
        temporary: &mut NamedTempFile,
        settings: &LifecycleSettings,
    ) -> io::Result<()> {
        if matches!(self.0, FailurePoint::Serialization) {
            temporary.write_all(b"{\"partial\":")?;
            return Err(io::Error::other("forced serialization failure"));
        }
        FileAtomicWriter.write_settings(temporary, settings)
    }

    fn persist(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()> {
        if matches!(self.0, FailurePoint::Persist) {
            return Err(io::Error::other("forced persist failure"));
        }
        FileAtomicWriter.persist(temporary, destination)
    }

    fn sync_parent(&self, parent: &Path) -> io::Result<()> {
        if matches!(self.0, FailurePoint::SyncParent) {
            return Err(io::Error::other("forced parent sync failure"));
        }
        FileAtomicWriter.sync_parent(parent)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SaveEvent {
    Write,
    Persist,
    SyncParent,
}

#[derive(Default)]
struct RecordingAtomicWriter {
    events: RefCell<Vec<SaveEvent>>,
}

impl AtomicWriter for RecordingAtomicWriter {
    fn write_settings(
        &self,
        temporary: &mut NamedTempFile,
        settings: &LifecycleSettings,
    ) -> io::Result<()> {
        self.events.borrow_mut().push(SaveEvent::Write);
        FileAtomicWriter.write_settings(temporary, settings)
    }

    fn persist(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()> {
        self.events.borrow_mut().push(SaveEvent::Persist);
        FileAtomicWriter.persist(temporary, destination)
    }

    fn sync_parent(&self, parent: &Path) -> io::Result<()> {
        self.events.borrow_mut().push(SaveEvent::SyncParent);
        FileAtomicWriter.sync_parent(parent)
    }
}

#[test]
fn successful_save_syncs_parent_directory_after_persist() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::from_path(path);
    let writer = RecordingAtomicWriter::default();

    store
        .save_with_writer(&LifecycleSettings::default(), &writer)
        .unwrap();

    assert_eq!(
        *writer.events.borrow(),
        [SaveEvent::Write, SaveEvent::Persist, SaveEvent::SyncParent]
    );
}

#[test]
fn serialization_failure_preserves_the_prior_valid_file() {
    assert_forced_failure_preserves_prior_file(FailurePoint::Serialization);
}

#[test]
fn persist_failure_preserves_the_prior_valid_file() {
    assert_forced_failure_preserves_prior_file(FailurePoint::Persist);
}

#[test]
fn parent_sync_failure_reports_that_replacement_was_already_committed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::from_path(&path);
    let mut original = LifecycleSettings::default();
    original.selected_steam_app_ids.insert(620);
    store.save(&original).unwrap();
    let mut replacement = LifecycleSettings::default();
    replacement.selected_steam_app_ids.insert(1_623_730);

    let error = store
        .save_with_writer(&replacement, &FailingAtomicWriter(FailurePoint::SyncParent))
        .unwrap_err();

    assert!(settings_save_was_committed(&error));
    assert_eq!(store.load().settings, replacement);
}

fn assert_forced_failure_preserves_prior_file(failure: FailurePoint) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::from_path(&path);
    let mut original = LifecycleSettings::default();
    original.selected_steam_app_ids.insert(620);
    store.save(&original).unwrap();
    let original_bytes = fs::read(&path).unwrap();
    let mut replacement = LifecycleSettings::default();
    replacement.manual_games.push(ManualGame {
        id: "local.portal".into(),
        name: "Portal".into(),
        executable: PathBuf::from("/games/portal"),
    });

    let error = store
        .save_with_writer(&replacement, &FailingAtomicWriter(failure))
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(fs::read(path).unwrap(), original_bytes);
    assert_eq!(store.load().settings, original);
}

#[test]
fn settings_error_is_an_io_error_source_when_save_validation_fails() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::from_path(temp.path().join("settings.json"));
    let mut invalid = LifecycleSettings::default();
    invalid.selected_steam_app_ids.insert(0);

    let error = store.save(&invalid).unwrap_err();

    assert!(matches!(
        error.get_ref().and_then(|source| source.downcast_ref()),
        Some(SettingsError::ZeroSteamAppId)
    ));
}

struct PermissionDeniedWriter;

impl Write for PermissionDeniedWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "forced permission failure",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn serde_writer_preserves_the_underlying_io_error_kind() {
    let error = write_settings_json(&mut PermissionDeniedWriter, &LifecycleSettings::default())
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

fn enabled_settings_json() -> Vec<u8> {
    let settings = LifecycleSettings {
        enabled: true,
        ..LifecycleSettings::default()
    };
    serde_json::to_vec(&settings).unwrap()
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &[u8]) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(unix)]
fn make_fifo(path: &Path) {
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `path` is a live, NUL-terminated C string for this call, and mode is valid.
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());
}
