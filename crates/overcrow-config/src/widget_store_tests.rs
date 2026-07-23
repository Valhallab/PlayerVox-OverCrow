use std::{
    cell::RefCell,
    ffi::OsStr,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::{
    ffi::CString,
    os::unix::{
        ffi::OsStrExt,
        fs::{PermissionsExt, symlink},
    },
};

use serde_json::{Value, json};
use tempfile::NamedTempFile;

use super::{AtomicWriter, FileAtomicWriter, WIDGET_MAX_BYTES, WidgetSettingsStore, widget_paths};
use crate::{WIDGET_SCHEMA_VERSION, WidgetId, WidgetPosition, WidgetProfile, WidgetProfileError};

fn profile_json() -> Value {
    serde_json::to_value(WidgetProfile::default()).unwrap()
}

#[test]
fn widget_ids_are_the_exact_stable_catalog() {
    assert_eq!(
        WidgetId::ALL,
        [
            WidgetId::Session,
            WidgetId::Clock,
            WidgetId::Performance,
            WidgetId::ManualStopwatch,
            WidgetId::Media,
            WidgetId::Notes,
            WidgetId::WarframeStatus,
            WidgetId::WarframeFissures,
            WidgetId::WarframeMarket,
            WidgetId::WarframeSortie,
            WidgetId::WarframeInvasions,
        ]
    );
}

#[test]
fn defaults_keep_only_the_existing_session_widget_enabled() {
    let profile = WidgetProfile::default();

    assert_eq!(profile.schema_version, WIDGET_SCHEMA_VERSION);
    assert!(profile.session.enabled);
    assert!(!profile.session.show_in_passive);
    for id in WidgetId::ALL
        .into_iter()
        .filter(|id| *id != WidgetId::Session)
    {
        assert!(!profile.settings(id).enabled);
    }
    assert!(!profile.clock.show_in_passive);
    assert!(!profile.performance.show_in_passive);
    assert!(!profile.manual_stopwatch.show_in_passive);
    assert!(!profile.media.show_in_passive);
    assert!(!profile.notes.show_in_passive);
    assert!(profile.warframe_status.show_in_passive);
    assert!(profile.warframe_fissures.show_in_passive);
    assert!(!profile.warframe_market.show_in_passive);
    assert!(profile.warframe_sortie.show_in_passive);
    assert!(profile.warframe_invasions.show_in_passive);
}

#[test]
fn widget_defaults_are_stable_and_pairwise_distinct() {
    let profile = WidgetProfile::default();
    let expected = [
        (WidgetId::Session, WidgetPosition { x: 0.0, y: 0.0 }),
        (WidgetId::Clock, WidgetPosition { x: 1.0, y: 0.0 }),
        (WidgetId::Performance, WidgetPosition { x: 0.0, y: 1.0 }),
        (WidgetId::ManualStopwatch, WidgetPosition { x: 1.0, y: 1.0 }),
        (WidgetId::Media, WidgetPosition { x: 0.5, y: 0.0 }),
        (WidgetId::Notes, WidgetPosition { x: 0.5, y: 1.0 }),
        (WidgetId::WarframeStatus, WidgetPosition { x: 0.5, y: 0.12 }),
        (
            WidgetId::WarframeFissures,
            WidgetPosition { x: 1.0, y: 0.45 },
        ),
        (WidgetId::WarframeMarket, WidgetPosition { x: 0.0, y: 0.45 }),
        (WidgetId::WarframeSortie, WidgetPosition { x: 0.0, y: 0.18 }),
        (
            WidgetId::WarframeInvasions,
            WidgetPosition { x: 1.0, y: 0.72 },
        ),
    ];

    for (index, (id, position)) in expected.into_iter().enumerate() {
        assert_eq!(profile.settings(id).position, position, "{id:?}");
        for (other_id, other_position) in expected.into_iter().skip(index + 1) {
            assert_ne!(position, other_position, "{id:?} and {other_id:?}");
        }
    }
}

#[test]
fn missing_transparent_background_defaults_to_opaque() {
    let legacy = json!({
        "schema_version": WIDGET_SCHEMA_VERSION,
        "session": {
            "enabled": true,
            "show_in_passive": false,
            "position": { "x": 0.0, "y": 0.0 }
        },
        "clock": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 1.0, "y": 0.0 }
        },
        "performance": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 0.0, "y": 1.0 }
        },
        "manual_stopwatch": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 1.0, "y": 1.0 }
        },
        "media": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 0.5, "y": 0.0 }
        },
        "notes": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 0.5, "y": 1.0 }
        }
    });

    let profile: WidgetProfile = serde_json::from_value(legacy).unwrap();
    for id in WidgetId::ALL {
        assert!(
            !profile.settings(id).transparent_background,
            "{id:?} should default to an opaque panel"
        );
    }
}

#[test]
fn missing_warframe_fields_deserialize_to_defaults() {
    let legacy = json!({
        "schema_version": WIDGET_SCHEMA_VERSION,
        "session": {
            "enabled": true,
            "show_in_passive": false,
            "position": { "x": 0.0, "y": 0.0 }
        },
        "clock": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 1.0, "y": 0.0 }
        },
        "performance": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 0.0, "y": 1.0 }
        },
        "manual_stopwatch": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 1.0, "y": 1.0 }
        },
        "media": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 0.5, "y": 0.0 }
        },
        "notes": {
            "enabled": false,
            "show_in_passive": false,
            "position": { "x": 0.5, "y": 1.0 }
        }
    });

    let profile: WidgetProfile = serde_json::from_value(legacy).unwrap();
    assert!(!profile.warframe_status.enabled);
    assert!(profile.warframe_status.show_in_passive);
    assert!(!profile.warframe_fissures.enabled);
    assert!(profile.warframe_fissures.show_in_passive);
    assert!(!profile.warframe_market.enabled);
    assert!(!profile.warframe_market.show_in_passive);
    assert!(!profile.warframe_sortie.enabled);
    assert!(profile.warframe_sortie.show_in_passive);
    assert!(!profile.warframe_invasions.enabled);
    assert!(profile.warframe_invasions.show_in_passive);
}

#[test]
fn typed_accessors_cover_every_widget() {
    let mut profile = WidgetProfile::default();

    for (index, id) in WidgetId::ALL.into_iter().enumerate() {
        profile.settings_mut(id).position.x = index as f32 / 10.0;
    }
    for (index, id) in WidgetId::ALL.into_iter().enumerate() {
        assert_eq!(profile.settings(id).position.x, index as f32 / 10.0);
    }
}

#[test]
fn deserialization_rejects_unknown_fields_at_every_profile_level() {
    let mut profile = profile_json();
    profile["unexpected"] = json!(true);

    let mut settings = profile_json();
    settings["session"]["unexpected"] = json!(true);

    let mut position = profile_json();
    position["session"]["position"]["unexpected"] = json!(true);

    for invalid in [profile, settings, position] {
        assert!(serde_json::from_value::<WidgetProfile>(invalid).is_err());
    }
}

#[test]
fn validation_rejects_schema_and_invalid_or_non_finite_positions() {
    for version in [0, WIDGET_SCHEMA_VERSION + 1, u32::MAX] {
        let profile = WidgetProfile {
            schema_version: version,
            ..WidgetProfile::default()
        };
        assert_eq!(
            profile.validate(),
            Err(WidgetProfileError::UnsupportedSchemaVersion)
        );
    }

    for invalid in [-0.1, 1.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut profile = WidgetProfile::default();
        profile.media.position.x = invalid;
        assert_eq!(
            profile.validate(),
            Err(WidgetProfileError::InvalidPosition(WidgetId::Media))
        );
    }

    let mut invalid_y = WidgetProfile::default();
    invalid_y.notes.position.y = -0.1;
    assert_eq!(
        invalid_y.validate(),
        Err(WidgetProfileError::InvalidPosition(WidgetId::Notes))
    );
}

#[test]
fn inclusive_normalized_position_bounds_are_valid() {
    let mut profile = WidgetProfile::default();
    profile.session.position = WidgetPosition { x: 0.0, y: 1.0 };

    assert_eq!(profile.clone().validate(), Ok(profile));
}

#[test]
fn widget_paths_prefer_absolute_xdg_then_absolute_home() {
    assert_eq!(
        widget_paths(Some(OsStr::new("/xdg")), Some(OsStr::new("/home/player"))),
        (
            PathBuf::from("/xdg/overcrow/widgets.json"),
            PathBuf::from("/xdg/overcrow/overlay.json")
        )
    );
    assert_eq!(
        widget_paths(None, Some(OsStr::new("/home/player"))),
        (
            PathBuf::from("/home/player/.config/overcrow/widgets.json"),
            PathBuf::from("/home/player/.config/overcrow/overlay.json")
        )
    );
    assert_eq!(
        widget_paths(
            Some(OsStr::new("relative")),
            Some(OsStr::new("/home/player"))
        ),
        (
            PathBuf::from("/home/player/.config/overcrow/widgets.json"),
            PathBuf::from("/home/player/.config/overcrow/overlay.json")
        )
    );
    assert_eq!(widget_paths(None, None), (PathBuf::new(), PathBuf::new()));
}

#[test]
fn missing_widget_and_legacy_files_return_defaults_without_a_warning() {
    let temp = tempfile::tempdir().unwrap();

    let load = WidgetSettingsStore::from_paths(
        temp.path().join("widgets.json"),
        temp.path().join("overlay.json"),
    )
    .load();

    assert_eq!(load.profile, WidgetProfile::default());
    assert!(load.warning.is_none());
}

#[test]
fn saved_profile_round_trips_as_private_pretty_json() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("nested/overcrow/widgets.json");
    let store = WidgetSettingsStore::from_paths(&current, temp.path().join("overlay.json"));
    let mut profile = WidgetProfile::default();
    profile.notes.enabled = true;

    store.save(&profile).unwrap();

    assert_eq!(store.load().profile, profile);
    let bytes = fs::read(&current).unwrap();
    assert!(bytes.ends_with(b"\n"));
    assert!(bytes.windows(4).any(|window| window == b"\n  \""));
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(current).unwrap().permissions().mode() & 0o7777,
        0o600
    );
}

#[test]
fn oversized_current_profile_is_rejected_with_a_warning() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("widgets.json");
    write_private(&current, &vec![b' '; WIDGET_MAX_BYTES + 1]);

    let load = WidgetSettingsStore::from_paths(&current, temp.path().join("overlay.json")).load();

    assert_eq!(load.profile, WidgetProfile::default());
    assert!(load.warning.unwrap().contains("too large"));
}

#[cfg(unix)]
#[test]
fn unsafe_current_profiles_are_rejected_without_legacy_fallback() {
    for unsafe_kind in [
        UnsafeFile::Symlink,
        UnsafeFile::Fifo,
        UnsafeFile::PublicMode,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let current = temp.path().join("widgets.json");
        let legacy = temp.path().join("overlay.json");
        write_private(&legacy, &legacy_json(true, 0.25, 0.75));
        make_unsafe_file(&current, unsafe_kind);

        let load = WidgetSettingsStore::from_paths(&current, &legacy).load();

        assert_eq!(load.profile, WidgetProfile::default(), "{unsafe_kind:?}");
        assert!(load.warning.unwrap().contains("unsafe"));
        assert_eq!(fs::read(&legacy).unwrap(), legacy_json(true, 0.25, 0.75));
    }
}

#[test]
fn malformed_current_profile_is_not_replaced_or_migrated_from_legacy() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("widgets.json");
    let legacy = temp.path().join("overlay.json");
    write_private(&current, b"{");
    write_private(&legacy, &legacy_json(true, 0.25, 0.75));

    let load = WidgetSettingsStore::from_paths(&current, &legacy).load();

    assert_eq!(load.profile, WidgetProfile::default());
    assert!(load.warning.unwrap().contains("invalid"));
    assert_eq!(fs::read(&current).unwrap(), b"{");
    assert_eq!(fs::read(&legacy).unwrap(), legacy_json(true, 0.25, 0.75));
}

#[test]
fn legacy_overlay_preferences_migrate_session_fields_only() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("widgets.json");
    let legacy = temp.path().join("overlay.json");
    write_private(
        &legacy,
        br#"{"show_stopwatch_in_passive":true,"stopwatch_position":{"x":0.25,"y":0.75}}"#,
    );

    let load = WidgetSettingsStore::from_paths(&current, &legacy).load();

    assert_eq!(
        load.profile.session.position,
        WidgetPosition { x: 0.25, y: 0.75 }
    );
    assert!(load.profile.session.show_in_passive);
    assert!(!load.profile.clock.enabled);
    assert_eq!(
        load.profile.clock.position,
        WidgetPosition { x: 1.0, y: 0.0 }
    );
    assert_eq!(
        load.profile.performance.position,
        WidgetPosition { x: 0.0, y: 1.0 }
    );
    assert!(load.warning.is_none());
    assert!(!current.exists());
}

#[test]
fn legacy_migration_requires_the_exact_valid_two_field_schema() {
    for legacy_contents in [
        br#"{"show_stopwatch_in_passive":true}"#.as_slice(),
        br#"{"show_stopwatch_in_passive":true,"stopwatch_position":{"x":0.25,"y":0.75},"unknown":false}"#,
        br#"{"show_stopwatch_in_passive":true,"stopwatch_position":{"x":-0.1,"y":0.75}}"#,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("overlay.json");
        write_private(&legacy, legacy_contents);

        let load = WidgetSettingsStore::from_paths(temp.path().join("widgets.json"), &legacy).load();

        assert_eq!(load.profile, WidgetProfile::default());
        assert!(load.warning.unwrap().contains("legacy"));
        assert_eq!(fs::read(legacy).unwrap(), legacy_contents);
    }
}

#[cfg(unix)]
#[test]
fn unsafe_or_oversized_legacy_files_are_rejected_without_blocking() {
    for unsafe_kind in [
        UnsafeFile::Symlink,
        UnsafeFile::Fifo,
        UnsafeFile::PublicMode,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("overlay.json");
        make_unsafe_file(&legacy, unsafe_kind);

        let load =
            WidgetSettingsStore::from_paths(temp.path().join("widgets.json"), &legacy).load();

        assert_eq!(load.profile, WidgetProfile::default(), "{unsafe_kind:?}");
        assert!(load.warning.unwrap().contains("unsafe"));
    }

    let temp = tempfile::tempdir().unwrap();
    let legacy = temp.path().join("overlay.json");
    write_private(&legacy, &vec![b' '; WIDGET_MAX_BYTES + 1]);
    let load = WidgetSettingsStore::from_paths(temp.path().join("widgets.json"), legacy).load();
    assert_eq!(load.profile, WidgetProfile::default());
    assert!(load.warning.unwrap().contains("too large"));
}

#[test]
fn save_atomically_replaces_an_existing_profile() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("widgets.json");
    let store = WidgetSettingsStore::from_paths(&current, temp.path().join("overlay.json"));
    store.save(&WidgetProfile::default()).unwrap();
    let original_bytes = fs::read(&current).unwrap();
    let mut replacement = WidgetProfile::default();
    replacement.performance.enabled = true;
    replacement.performance.position = WidgetPosition { x: 0.4, y: 0.6 };

    store.save(&replacement).unwrap();

    assert_ne!(fs::read(&current).unwrap(), original_bytes);
    assert_eq!(store.load().profile, replacement);
}

#[test]
fn invalid_profile_is_not_saved_over_a_valid_file() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("widgets.json");
    let store = WidgetSettingsStore::from_paths(&current, temp.path().join("overlay.json"));
    store.save(&WidgetProfile::default()).unwrap();
    let original_bytes = fs::read(&current).unwrap();
    let mut invalid = WidgetProfile::default();
    invalid.session.position.y = f32::NAN;

    let error = store.save(&invalid).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read(current).unwrap(), original_bytes);
}

#[derive(Clone, Copy)]
enum FailurePoint {
    Serialization,
    Persist,
}

struct FailingAtomicWriter(FailurePoint);

impl AtomicWriter for FailingAtomicWriter {
    fn write_profile(
        &self,
        temporary: &mut NamedTempFile,
        profile: &WidgetProfile,
    ) -> io::Result<()> {
        if matches!(self.0, FailurePoint::Serialization) {
            temporary.write_all(b"{\"partial\":")?;
            return Err(io::Error::other("forced serialization failure"));
        }
        FileAtomicWriter.write_profile(temporary, profile)
    }

    fn persist(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()> {
        if matches!(self.0, FailurePoint::Persist) {
            return Err(io::Error::other("forced persist failure"));
        }
        FileAtomicWriter.persist(temporary, destination)
    }

    fn sync_parent(&self, parent: &Path) -> io::Result<()> {
        FileAtomicWriter.sync_parent(parent)
    }
}

#[test]
fn serialization_and_persist_failures_preserve_the_prior_profile() {
    for failure in [FailurePoint::Serialization, FailurePoint::Persist] {
        let temp = tempfile::tempdir().unwrap();
        let current = temp.path().join("widgets.json");
        let store = WidgetSettingsStore::from_paths(&current, temp.path().join("overlay.json"));
        store.save(&WidgetProfile::default()).unwrap();
        let original_bytes = fs::read(&current).unwrap();
        let mut replacement = WidgetProfile::default();
        replacement.media.enabled = true;

        store
            .save_with_writer(&replacement, &FailingAtomicWriter(failure))
            .unwrap_err();

        assert_eq!(fs::read(&current).unwrap(), original_bytes);
        assert_eq!(store.load().profile, WidgetProfile::default());
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
    fn write_profile(
        &self,
        temporary: &mut NamedTempFile,
        profile: &WidgetProfile,
    ) -> io::Result<()> {
        self.events.borrow_mut().push(SaveEvent::Write);
        FileAtomicWriter.write_profile(temporary, profile)
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
fn successful_save_persists_then_syncs_the_parent_directory() {
    let temp = tempfile::tempdir().unwrap();
    let store = WidgetSettingsStore::from_paths(
        temp.path().join("widgets.json"),
        temp.path().join("overlay.json"),
    );
    let writer = RecordingAtomicWriter::default();

    store
        .save_with_writer(&WidgetProfile::default(), &writer)
        .unwrap();

    assert_eq!(
        *writer.events.borrow(),
        [SaveEvent::Write, SaveEvent::Persist, SaveEvent::SyncParent]
    );
}

fn legacy_json(show_in_passive: bool, x: f32, y: f32) -> Vec<u8> {
    format!(
        r#"{{"show_stopwatch_in_passive":{show_in_passive},"stopwatch_position":{{"x":{x},"y":{y}}}}}"#
    )
    .into_bytes()
}

fn write_private(path: &Path, contents: &[u8]) {
    fs::write(path, contents).unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
enum UnsafeFile {
    Symlink,
    Fifo,
    PublicMode,
}

#[cfg(unix)]
fn make_unsafe_file(path: &Path, kind: UnsafeFile) {
    match kind {
        UnsafeFile::Symlink => {
            let target = path.with_extension("target");
            write_private(
                &target,
                &serde_json::to_vec(&WidgetProfile::default()).unwrap(),
            );
            symlink(target, path).unwrap();
        }
        UnsafeFile::Fifo => {
            let path = CString::new(path.as_os_str().as_bytes()).unwrap();
            // SAFETY: `path` is a live, NUL-terminated C string and mode is valid.
            let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
            assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());
        }
        UnsafeFile::PublicMode => {
            fs::write(path, serde_json::to_vec(&WidgetProfile::default()).unwrap()).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
}
