use std::{ffi::OsStr, fs, io::Write, os::unix::fs::PermissionsExt, path::PathBuf};

use tempfile::tempdir;

use super::{
    FissureEra, StatusRow, WARFRAME_ACTIVITY_DONE_MAX, WARFRAME_MARKET_QUERY_MAX_CHARS,
    WARFRAME_PREFS_MAX_BYTES, WARFRAME_PREFS_SCHEMA_VERSION, WARFRAME_STEAM_APP_ID, WarframePrefs,
    WarframePrefsStore, warframe_prefs_path,
};

#[test]
fn warframe_steam_app_id_is_stable() {
    assert_eq!(WARFRAME_STEAM_APP_ID, 230_410);
}

#[test]
fn defaults_are_safe() {
    let prefs = WarframePrefs::default();
    assert_eq!(prefs.schema_version, WARFRAME_PREFS_SCHEMA_VERSION);
    assert!(prefs.fissure_eras.is_empty());
    assert!(prefs.show_normal);
    assert!(prefs.show_steel_path);
    assert!(prefs.show_railjack);
    assert!(!prefs.status_horizontal);
    assert!(prefs.show_status_seconds);
    assert!(prefs.show_fissure_seconds);
    assert!(prefs.show_fissure_node);
    assert!(
        StatusRow::ALL
            .into_iter()
            .all(|row| prefs.status_row_visible(row))
    );
    assert!(prefs.last_market_query.is_empty());
    assert!(prefs.invasion_hide_completed);
    assert!(prefs.invasion_push_done_down);
    assert!(!prefs.invasion_compact);
    assert!(prefs.invasion_resource_filter.is_empty());
    assert!(prefs.invasion_reward_watchlist.is_empty());
    assert!(!prefs.nightwave_daily_only);
    assert!(!prefs.nightwave_show_expired);
    assert!(prefs.show_activity_seconds);
    assert!(prefs.activity_done.is_empty());
}

#[test]
fn validation_rejects_overlong_query() {
    let prefs = WarframePrefs {
        last_market_query: "x".repeat(WARFRAME_MARKET_QUERY_MAX_CHARS + 1),
        ..WarframePrefs::default()
    };
    assert!(prefs.validate().is_err());
}

#[test]
fn validation_rejects_no_source_selected() {
    let prefs = WarframePrefs {
        show_normal: false,
        show_steel_path: false,
        show_railjack: false,
        ..WarframePrefs::default()
    };
    assert!(prefs.validate().is_err());
}

#[test]
fn missing_file_loads_defaults_without_warning() {
    let directory = tempdir().unwrap();
    let store = WarframePrefsStore::from_path(directory.path().join("warframe.json"));
    let load = store.load();
    assert_eq!(load.prefs, WarframePrefs::default());
    assert!(load.warning.is_none());
}

#[test]
fn save_round_trips_private_json() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("warframe.json");
    let store = WarframePrefsStore::from_path(&path);
    let prefs = WarframePrefs {
        show_normal: false,
        show_steel_path: true,
        show_railjack: false,
        fissure_eras: vec![FissureEra::Lith, FissureEra::Axi],
        status_horizontal: true,
        show_status_zariman: false,
        last_market_query: "valkyr".to_owned(),
        ..WarframePrefs::default()
    };
    store.save(&prefs).unwrap();

    let metadata = fs::metadata(&path).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    let load = store.load();
    assert_eq!(load.prefs, prefs);
    assert!(load.warning.is_none());
}

#[test]
fn save_persists_the_validated_normalized_candidate() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("warframe.json");
    let store = WarframePrefsStore::from_path(&path);
    let prefs = WarframePrefs {
        activity_done: vec!["sortie:1:0".into(), "sortie:1:0".into()],
        ..WarframePrefs::default()
    };

    store.save(&prefs).unwrap();

    let persisted: WarframePrefs = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(persisted.activity_done, vec!["sortie:1:0"]);
}

#[test]
fn oversized_save_does_not_replace_a_valid_file() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("warframe.json");
    let store = WarframePrefsStore::from_path(&path);
    store.save(&WarframePrefs::default()).unwrap();
    let before = fs::read(&path).unwrap();

    let unique_entries = |count: usize| {
        (0..count)
            .map(|index| format!("{index:03}{}", "🦀".repeat(93)))
            .collect::<Vec<_>>()
    };
    let oversized = WarframePrefs {
        activity_done: unique_entries(128),
        invasion_resource_filter: unique_entries(24),
        invasion_reward_watchlist: unique_entries(24),
        ..WarframePrefs::default()
    };
    let normalized = oversized.clone().validate().unwrap();
    let mut serialized = serde_json::to_vec_pretty(&normalized).unwrap();
    serialized.push(b'\n');
    assert!(serialized.len() > WARFRAME_PREFS_MAX_BYTES);

    let error = store.save(&oversized).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn legacy_prefs_without_status_fields_load_defaults() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("warframe.json");
    {
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(
            br#"{
  "schema_version": 1,
  "fissure_eras": [],
  "show_normal": true,
  "show_steel_path": true,
  "show_railjack": true,
  "last_market_query": ""
}"#,
        )
        .unwrap();
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    let load = WarframePrefsStore::from_path(&path).load();
    assert!(load.warning.is_none());
    assert!(!load.prefs.status_horizontal);
    assert!(load.prefs.show_status_cetus);
}

#[test]
fn malformed_file_fails_closed_without_overwrite() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("warframe.json");
    {
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(br#"{"schema_version":1,"nope":true}"#)
            .unwrap();
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    let before = fs::read(&path).unwrap();
    let store = WarframePrefsStore::from_path(&path);
    let load = store.load();
    assert_eq!(load.prefs, WarframePrefs::default());
    assert!(load.warning.is_some());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn void_modifier_maps_to_era() {
    assert_eq!(
        FissureEra::from_void_modifier("VoidT1"),
        Some(FissureEra::Lith)
    );
    assert_eq!(
        FissureEra::from_void_modifier("VoidT6"),
        Some(FissureEra::Omni)
    );
    assert_eq!(FissureEra::from_void_modifier("Other"), None);
}

#[test]
fn prefs_paths_prefer_absolute_xdg() {
    assert_eq!(
        warframe_prefs_path(Some(OsStr::new("/cfg")), None),
        PathBuf::from("/cfg/overcrow/warframe.json")
    );
}

#[test]
fn pruning_expired_completion_keys_makes_room_for_a_current_insertion() {
    let mut prefs = WarframePrefs {
        activity_done: (0..WARFRAME_ACTIVITY_DONE_MAX)
            .map(|index| format!("sortie:{index}:0"))
            .collect(),
        ..WarframePrefs::default()
    };
    let current = vec![
        "sortie:2000:0".to_owned(),
        "archon:3000:0".to_owned(),
        "invasion:provider-object-a".to_owned(),
    ];

    prefs.prune_activity_done(&current);
    prefs.toggle_activity_done(&current[0]);

    assert_eq!(prefs.activity_done, vec![current[0].clone()]);
}

#[test]
fn pruning_keeps_only_exact_current_keys_and_discards_unknown_legacy_entries() {
    let current = vec![
        "sortie:2000:0".to_owned(),
        "archon:3000:0".to_owned(),
        "invasion:provider-object-a".to_owned(),
    ];
    let mut prefs = WarframePrefs {
        activity_done: vec![
            "unknown:legacy".to_owned(),
            "sortie:1000:0".to_owned(),
            current[2].clone(),
            current[0].clone(),
        ],
        ..WarframePrefs::default()
    };

    prefs.prune_activity_done(&current);

    assert_eq!(
        prefs.activity_done,
        vec![current[2].clone(), current[0].clone()]
    );
}

#[test]
fn full_completion_list_evicts_the_oldest_entry_for_a_valid_new_key() {
    let mut prefs = WarframePrefs {
        activity_done: (0..WARFRAME_ACTIVITY_DONE_MAX)
            .map(|index| format!("legacy:{index:03}"))
            .collect(),
        ..WarframePrefs::default()
    };

    prefs.toggle_activity_done("invasion:current");

    assert_eq!(prefs.activity_done.len(), WARFRAME_ACTIVITY_DONE_MAX);
    assert!(!prefs.activity_is_done("legacy:000"));
    assert!(prefs.activity_is_done("legacy:001"));
    assert_eq!(
        prefs.activity_done.last().map(String::as_str),
        Some("invasion:current")
    );
}
