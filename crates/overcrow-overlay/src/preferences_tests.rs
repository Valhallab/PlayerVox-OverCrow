use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use overcrow_config::{WidgetPosition, WidgetProfile};

use super::PreferenceStore;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "overcrow-overlay-preferences-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("preference fixture directory");
        Self { root }
    }

    fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative)
    }

    fn write_private(&self, relative: impl AsRef<Path>, contents: &[u8]) -> PathBuf {
        let path = self.path(relative);
        fs::write(&path, contents).expect("preference fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("private preference permissions");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn missing_preferences_use_the_shared_widget_profile_defaults() {
    let fixture = Fixture::new();
    let store =
        PreferenceStore::from_paths(fixture.path("widgets.json"), fixture.path("overlay.json"));

    let load = store.load();

    assert_eq!(load.profile, WidgetProfile::default());
    assert_eq!(load.warning, None);
}

#[test]
fn legacy_session_preferences_are_loaded_through_the_shared_store() {
    let fixture = Fixture::new();
    let legacy = fixture.write_private(
        "overlay.json",
        br#"{"show_stopwatch_in_passive":true,"stopwatch_position":{"x":0.25,"y":0.75}}"#,
    );
    let store = PreferenceStore::from_paths(fixture.path("widgets.json"), legacy);

    let load = store.load();

    assert!(load.profile.session.enabled);
    assert!(load.profile.session.show_in_passive);
    assert_eq!(
        load.profile.session.position,
        WidgetPosition { x: 0.25, y: 0.75 }
    );
    assert_eq!(load.warning, None);
}

#[test]
fn the_wrapper_persists_the_complete_shared_profile() {
    let fixture = Fixture::new();
    let current = fixture.path("overcrow/widgets.json");
    let store = PreferenceStore::from_paths(&current, fixture.path("overlay.json"));
    let mut profile = WidgetProfile::default();
    profile.clock.enabled = true;
    profile.clock.show_in_passive = true;
    profile.clock.position = WidgetPosition { x: 0.4, y: 0.6 };

    store.save(&profile).expect("widget profile save");

    assert_eq!(store.load().profile, profile);
    assert_eq!(
        fs::metadata(current)
            .expect("widget profile metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn rejected_shared_profiles_keep_the_warning() {
    let fixture = Fixture::new();
    let current = fixture.write_private("widgets.json", b"{");
    let store = PreferenceStore::from_paths(current, fixture.path("overlay.json"));

    let load = store.load();

    assert_eq!(load.profile, WidgetProfile::default());
    assert!(load.warning.is_some());
}

#[test]
fn saving_without_a_config_home_is_a_non_destructive_error() {
    let store = PreferenceStore::from_paths(PathBuf::new(), PathBuf::new());

    let error = store
        .save(&WidgetProfile::default())
        .expect_err("missing config home must reject persistence");

    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}
