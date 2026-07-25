use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

use serde_json::json;

use super::{
    TWITCH_CHANNEL_MAX_CHARS, TWITCH_FAVORITES_MAX, TWITCH_PASSIVE_LIFETIME_MAX_SECS,
    TWITCH_PASSIVE_LIFETIME_MIN_SECS, TWITCH_PREFS_MAX_BYTES, TWITCH_PREFS_SCHEMA_VERSION,
    TwitchPrefs, TwitchPrefsError, TwitchPrefsStore, normalize_twitch_channel, twitch_prefs_path,
};

fn write_private(path: &std::path::Path, contents: &[u8]) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn twitch_prefs_normalizes_exact_valid_channel_logins() {
    assert_eq!(
        normalize_twitch_channel("  #Player_Vox  ").unwrap(),
        "player_vox"
    );
    assert_eq!(normalize_twitch_channel("a").unwrap(), "a");
    assert_eq!(
        normalize_twitch_channel(&"Z".repeat(TWITCH_CHANNEL_MAX_CHARS)).unwrap(),
        "z".repeat(TWITCH_CHANNEL_MAX_CHARS)
    );

    for invalid in [
        "",
        "#",
        "two channels",
        "hyphen-name",
        "é",
        "name\nother",
        "##channel",
    ] {
        assert_eq!(
            normalize_twitch_channel(invalid),
            Err(TwitchPrefsError::InvalidChannel),
            "{invalid:?}"
        );
    }
    assert_eq!(
        normalize_twitch_channel(&"x".repeat(TWITCH_CHANNEL_MAX_CHARS + 1)),
        Err(TwitchPrefsError::InvalidChannel)
    );
}

#[test]
fn twitch_prefs_validation_preserves_order_and_deduplicates_favorites() {
    let prefs = TwitchPrefs {
        active_channel: Some("  #PlayerVox ".to_owned()),
        favorites: vec![
            "Warframe".to_owned(),
            "playerVox".to_owned(),
            "warframe".to_owned(),
        ],
        ..TwitchPrefs::default()
    }
    .validate()
    .unwrap();

    assert_eq!(prefs.active_channel.as_deref(), Some("playervox"));
    assert_eq!(prefs.favorites, ["warframe", "playervox"]);
}

#[test]
fn twitch_prefs_rejects_too_many_favorites_and_invalid_lifetime() {
    let too_many = TwitchPrefs {
        favorites: (0..=TWITCH_FAVORITES_MAX)
            .map(|index| format!("channel_{index}"))
            .collect(),
        ..TwitchPrefs::default()
    };
    assert_eq!(too_many.validate(), Err(TwitchPrefsError::TooManyFavorites));

    for lifetime in [
        TWITCH_PASSIVE_LIFETIME_MIN_SECS - 1,
        TWITCH_PASSIVE_LIFETIME_MAX_SECS + 1,
    ] {
        let prefs = TwitchPrefs {
            passive_lifetime_secs: lifetime,
            ..TwitchPrefs::default()
        };
        assert_eq!(
            prefs.validate(),
            Err(TwitchPrefsError::InvalidPassiveLifetime)
        );
    }
}

#[test]
fn twitch_prefs_rejects_unknown_fields_and_schema_versions() {
    let unknown = json!({
        "schema_version": TWITCH_PREFS_SCHEMA_VERSION,
        "active_channel": null,
        "favorites": [],
        "passive_lifetime_secs": 30,
        "token": "must never be accepted"
    });
    assert!(serde_json::from_value::<TwitchPrefs>(unknown).is_err());

    let unsupported = TwitchPrefs {
        schema_version: TWITCH_PREFS_SCHEMA_VERSION + 1,
        ..TwitchPrefs::default()
    };
    assert_eq!(
        unsupported.validate(),
        Err(TwitchPrefsError::UnsupportedSchemaVersion)
    );
}

#[test]
fn twitch_prefs_store_round_trips_private_atomic_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config/overcrow/twitch.json");
    let store = TwitchPrefsStore::from_path(&path);
    let expected = TwitchPrefs {
        active_channel: Some("player_vox".to_owned()),
        favorites: vec!["warframe".to_owned(), "player_vox".to_owned()],
        passive_lifetime_secs: 45,
        ..TwitchPrefs::default()
    };

    store.save(&expected).unwrap();

    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let load = store.load();
    assert_eq!(load.warning, None);
    assert_eq!(load.prefs, expected);
}

#[test]
fn twitch_prefs_store_rejects_unsafe_or_oversized_files() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target.json");
    write_private(
        &target,
        &serde_json::to_vec(&TwitchPrefs::default()).unwrap(),
    );
    let link = temp.path().join("twitch.json");
    symlink(&target, &link).unwrap();

    let symlink_load = TwitchPrefsStore::from_path(&link).load();
    assert_eq!(symlink_load.prefs, TwitchPrefs::default());
    assert!(
        symlink_load
            .warning
            .unwrap()
            .contains("unsafe Twitch preferences")
    );

    let oversized = temp.path().join("oversized.json");
    write_private(&oversized, &vec![b' '; TWITCH_PREFS_MAX_BYTES + 1]);
    let oversized_load = TwitchPrefsStore::from_path(&oversized).load();
    assert_eq!(oversized_load.prefs, TwitchPrefs::default());
    assert!(oversized_load.warning.unwrap().contains("too large"));
}

#[test]
fn twitch_prefs_path_requires_an_absolute_environment_root() {
    assert_eq!(
        twitch_prefs_path(Some(OsStr::new("/tmp/config")), None),
        std::path::PathBuf::from("/tmp/config/overcrow/twitch.json")
    );
    assert_eq!(
        twitch_prefs_path(None, Some(OsStr::new("/home/user"))),
        std::path::PathBuf::from("/home/user/.config/overcrow/twitch.json")
    );
    assert_eq!(
        twitch_prefs_path(Some(OsStr::new("relative")), Some(OsStr::new("relative"))),
        std::path::PathBuf::new()
    );
}
