use std::{collections::BTreeSet, path::PathBuf};

use crate::{GameAllowlist, LifecycleSettings, ManualGame, ProcessIdentity};

fn enabled_settings() -> LifecycleSettings {
    LifecycleSettings {
        enabled: true,
        ..LifecycleSettings::default()
    }
}

fn settings_with_steam<const N: usize>(ids: [u32; N]) -> LifecycleSettings {
    LifecycleSettings {
        selected_steam_app_ids: BTreeSet::from(ids),
        ..enabled_settings()
    }
}

fn settings_with_manual(path: &str) -> LifecycleSettings {
    LifecycleSettings {
        manual_games: vec![ManualGame {
            id: "manual-game".to_owned(),
            name: "Manual game".to_owned(),
            executable: PathBuf::from(path),
        }],
        ..enabled_settings()
    }
}

fn identity_with_executable(path: &str) -> ProcessIdentity {
    ProcessIdentity {
        executable_chain: vec![PathBuf::from(path)],
        game_candidate: true,
        ..ProcessIdentity::default()
    }
}

#[test]
fn an_unselected_steam_game_is_rejected() {
    let allowlist = GameAllowlist::from_settings(&settings_with_steam([1_623_730]));
    let identity = ProcessIdentity {
        steam_app_id: Some(620),
        executable_chain: vec![PathBuf::from("/games/portal2")],
        game_candidate: true,
    };

    assert!(!allowlist.allows_identity(&identity));
}

#[test]
fn a_selected_steam_game_is_allowed_by_ancestry_identity() {
    let allowlist = GameAllowlist::from_settings(&settings_with_steam([620]));
    let identity = ProcessIdentity {
        steam_app_id: Some(620),
        executable_chain: vec![
            PathBuf::from("/games/portal2"),
            PathBuf::from("/usr/bin/steam-launch-wrapper"),
        ],
        game_candidate: false,
    };

    assert!(allowlist.allows_identity(&identity));
}

#[test]
fn manual_identity_requires_an_exact_canonical_path() {
    let allowlist = GameAllowlist::from_settings(&settings_with_manual("/games/portal2"));

    assert!(allowlist.allows_identity(&identity_with_executable("/games/portal2")));
    assert!(!allowlist.allows_identity(&identity_with_executable("/games/portal2-helper")));
}

#[test]
fn a_manual_ancestor_executable_is_allowed() {
    let allowlist = GameAllowlist::from_settings(&settings_with_manual("/games/launcher"));
    let identity = ProcessIdentity {
        executable_chain: vec![
            PathBuf::from("/games/game"),
            PathBuf::from("/games/launcher"),
        ],
        ..ProcessIdentity::default()
    };

    assert!(allowlist.allows_identity(&identity));
}

#[test]
fn zero_steam_ids_never_authorize_a_process() {
    let allowlist = GameAllowlist::from_settings(&settings_with_steam([0]));
    let identity = ProcessIdentity {
        steam_app_id: Some(0),
        ..ProcessIdentity::default()
    };

    assert!(!allowlist.allows_identity(&identity));
}

#[test]
fn generic_wine_metadata_does_not_authorize_a_process() {
    let allowlist = GameAllowlist::from_settings(&settings_with_manual("/games/portal2"));
    let identity = ProcessIdentity {
        executable_chain: vec![PathBuf::from("/usr/bin/wine64")],
        game_candidate: true,
        ..ProcessIdentity::default()
    };

    assert!(!allowlist.allows_identity(&identity));
}

#[test]
fn disabled_settings_produce_an_inert_allowlist() {
    let mut settings = settings_with_steam([620]);
    settings.enabled = false;
    let allowlist = GameAllowlist::from_settings(&settings);

    assert!(!allowlist.allows_identity(&ProcessIdentity {
        steam_app_id: Some(620),
        ..ProcessIdentity::default()
    }));
}

#[test]
fn malformed_settings_produce_an_inert_allowlist() {
    let mut settings = settings_with_steam([620]);
    settings.shortcut.accelerator = "invalid".to_owned();
    let allowlist = GameAllowlist::from_settings(&settings);

    assert!(!allowlist.allows_identity(&ProcessIdentity {
        steam_app_id: Some(620),
        ..ProcessIdentity::default()
    }));
}

#[test]
fn selected_process_scan_handles_multiple_simultaneous_identities() {
    let allowlist = GameAllowlist::from_settings(&settings_with_steam([620]));
    let processes = std::collections::HashMap::from([
        (
            99,
            ProcessIdentity {
                steam_app_id: Some(730),
                ..ProcessIdentity::default()
            },
        ),
        (
            42,
            ProcessIdentity {
                steam_app_id: Some(620),
                ..ProcessIdentity::default()
            },
        ),
    ]);

    assert!(allowlist.any_selected_process([99, 42], |pid| processes[&pid].clone()));
}

#[test]
fn selected_process_scan_sorts_and_deduplicates_before_classification() {
    let allowlist = GameAllowlist::from_settings(&settings_with_steam([620]));
    let mut callback_order = Vec::new();

    let selected = allowlist.any_selected_process([99, 42, 99, 7, 42], |pid| {
        callback_order.push(pid);
        ProcessIdentity::default()
    });

    assert!(!selected);
    assert_eq!(callback_order, [7, 42, 99]);
}

#[test]
fn selected_process_scan_reclassifies_a_reused_pid_once_per_snapshot() {
    let allowlist = GameAllowlist::from_settings(&settings_with_steam([620]));
    let mut current_identity = ProcessIdentity {
        steam_app_id: Some(620),
        ..ProcessIdentity::default()
    };
    let mut classifications = 0;

    let originally_selected = allowlist.any_selected_process([42, 42], |_| {
        classifications += 1;
        current_identity.clone()
    });

    current_identity.steam_app_id = Some(730);
    let replacement_selected = allowlist.any_selected_process([42, 42], |_| {
        classifications += 1;
        current_identity.clone()
    });

    assert!(originally_selected);
    assert!(!replacement_selected);
    assert_eq!(classifications, 2);
}

#[test]
fn candidate_metadata_alone_does_not_make_any_process_selected() {
    let allowlist = GameAllowlist::from_settings(&settings_with_steam([620]));
    let generic_candidates = std::collections::HashMap::from([
        (
            7,
            ProcessIdentity {
                executable_chain: vec![PathBuf::from("/usr/bin/proton")],
                game_candidate: true,
                ..ProcessIdentity::default()
            },
        ),
        (
            8,
            ProcessIdentity {
                executable_chain: vec![PathBuf::from("/games/unselected.exe")],
                game_candidate: true,
                ..ProcessIdentity::default()
            },
        ),
    ]);

    assert!(!allowlist.any_selected_process([7, 8], |pid| generic_candidates[&pid].clone()));
}
