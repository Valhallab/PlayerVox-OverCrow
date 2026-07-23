use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::PermissionsExt,
    },
    path::{Path, PathBuf},
};

use overcrow_config::{LifecycleSettings, ManualGame, SettingsLoad, SettingsStore};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::{
    ControlModel, DiscoveryReport, NativePathValidator, PathValidator, SelectionError, SteamGame,
};

struct FakeValidator {
    results: BTreeMap<PathBuf, Result<PathBuf, SelectionError>>,
}

impl FakeValidator {
    fn accepting(entries: impl IntoIterator<Item = (PathBuf, PathBuf)>) -> Self {
        Self {
            results: entries
                .into_iter()
                .map(|(requested, canonical)| (requested, Ok(canonical)))
                .collect(),
        }
    }

    fn rejecting(path: PathBuf, error: SelectionError) -> Self {
        Self {
            results: BTreeMap::from([(path, Err(error))]),
        }
    }
}

impl PathValidator for FakeValidator {
    fn canonical_executable(&self, path: &Path) -> Result<PathBuf, SelectionError> {
        self.results.get(path).cloned().unwrap_or({
            Err(SelectionError::ExecutableValidationFailed(
                io::ErrorKind::NotFound,
            ))
        })
    }
}

fn game(app_id: u32, name: &str) -> SteamGame {
    SteamGame {
        app_id,
        name: name.to_owned(),
        install_dir: PathBuf::from(format!("/steam/{app_id}")),
        icon: None,
    }
}

fn model_with<V>(settings: LifecycleSettings, games: Vec<SteamGame>, validator: V) -> ControlModel
where
    V: PathValidator + 'static,
{
    ControlModel::new(
        SettingsLoad {
            settings,
            warning: None,
        },
        DiscoveryReport {
            games,
            warnings: Vec::new(),
        },
        validator,
    )
}

fn loaded_model<V>(
    manual_games: Vec<ManualGame>,
    settings_warning: Option<&str>,
    validator: V,
) -> ControlModel
where
    V: PathValidator + 'static,
{
    ControlModel::new(
        SettingsLoad {
            settings: LifecycleSettings {
                enabled: true,
                manual_games,
                ..LifecycleSettings::default()
            },
            warning: settings_warning.map(str::to_owned),
        },
        DiscoveryReport::default(),
        validator,
    )
}

fn expected_manual_id(path: &Path) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let digest = Sha256::digest(path.as_os_str().as_bytes());
    let mut id = String::from("local.");
    for byte in &digest[..16] {
        id.push(HEX[usize::from(byte >> 4)] as char);
        id.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    id
}

fn loaded_manual_game(name: &str, executable: PathBuf) -> ManualGame {
    ManualGame {
        id: expected_manual_id(&executable),
        name: name.to_owned(),
        executable,
    }
}

fn assert_one_loaded_game_was_dropped(model: &ControlModel) {
    assert!(model.settings.manual_games.is_empty());
    assert!(
        model
            .settings_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("Dropped 1 invalid manual game selection"))
    );
    assert!(!model.settings.enabled);
}

fn steam_model() -> ControlModel {
    model_with(
        LifecycleSettings::default(),
        vec![game(620, "Portal 2"), game(1_623_730, "Palworld")],
        FakeValidator::accepting([]),
    )
}

#[test]
fn search_matches_game_names_case_insensitively() {
    let mut model = steam_model();

    model.set_search("pAlWoRLD");

    assert_eq!(
        model
            .visible_games()
            .into_iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![1_623_730]
    );
}

#[test]
fn search_uses_full_unicode_case_folding_for_sharp_s() {
    let mut model = model_with(
        LifecycleSettings::default(),
        vec![game(10, "Straße")],
        FakeValidator::accepting([]),
    );

    model.set_search("STRASSE");

    assert_eq!(
        model
            .visible_games()
            .into_iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![10]
    );
}

#[test]
fn search_folds_all_greek_sigma_variants_equivalently() {
    let mut model = model_with(
        LifecycleSettings::default(),
        vec![game(11, "ΜΕΣΟΣ")],
        FakeValidator::accepting([]),
    );

    for search in ["μεσοσ", "μεσος"] {
        model.set_search(search);
        assert_eq!(
            model
                .visible_games()
                .into_iter()
                .map(|game| game.app_id)
                .collect::<Vec<_>>(),
            vec![11]
        );
    }
}

#[test]
fn clearing_search_restores_all_games() {
    let mut model = steam_model();
    model.set_search("portal");

    model.set_search("   ");

    assert_eq!(model.visible_games().len(), 2);
}

#[test]
fn known_steam_games_can_be_selected_and_deselected() {
    let mut model = steam_model();

    model.set_steam_selected(620, true);
    assert!(model.settings.selected_steam_app_ids.contains(&620));

    model.set_steam_selected(620, false);
    assert!(!model.settings.selected_steam_app_ids.contains(&620));
}

#[test]
fn unknown_steam_ids_are_not_newly_selected() {
    let mut model = steam_model();

    model.set_steam_selected(999, true);

    assert!(!model.settings.selected_steam_app_ids.contains(&999));
}

#[test]
fn loaded_unknown_steam_ids_survive_until_explicitly_deselected() {
    let settings = LifecycleSettings {
        selected_steam_app_ids: BTreeSet::from([999]),
        ..LifecycleSettings::default()
    };
    let mut model = model_with(
        settings,
        vec![game(620, "Portal 2")],
        FakeValidator::accepting([]),
    );

    assert!(model.settings.selected_steam_app_ids.contains(&999));
    model.set_steam_selected(620, true);
    assert!(model.settings.selected_steam_app_ids.contains(&999));

    model.set_steam_selected(999, false);
    assert!(!model.settings.selected_steam_app_ids.contains(&999));
}

#[test]
fn constructor_preserves_valid_loaded_lifecycle_authority() {
    let settings = LifecycleSettings {
        enabled: true,
        ..LifecycleSettings::default()
    };

    let model = model_with(settings, Vec::new(), FakeValidator::accepting([]));

    assert!(model.settings.enabled);
}

#[test]
fn saving_selections_preserves_lifecycle_authority() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::from_path(temp.path().join("overcrow/settings.json"));
    let mut model = steam_model();
    model.set_steam_selected(1_623_730, true);
    model.settings.enabled = true;

    model.save_selections(&store).unwrap();

    let saved = store.load().settings;
    assert!(saved.enabled);
    assert!(saved.selected_steam_app_ids.contains(&1_623_730));
}

#[test]
fn constructor_propagates_discovery_and_settings_warnings() {
    let model = ControlModel::new(
        SettingsLoad {
            settings: LifecycleSettings::default(),
            warning: Some("settings warning".to_owned()),
        },
        DiscoveryReport {
            games: vec![game(620, "Portal 2")],
            warnings: vec!["manifest warning".to_owned()],
        },
        FakeValidator::accepting([]),
    );

    assert_eq!(model.settings_warning.as_deref(), Some("settings warning"));
    assert_eq!(model.discovery_warnings, ["manifest warning"]);
    assert_eq!(model.games.len(), 1);
}

#[test]
fn valid_canonical_loaded_manual_games_are_trimmed_retained_and_saveable() {
    let path = PathBuf::from("/games/portal");
    let original = loaded_manual_game("  Portal  ", path.clone());
    let mut expected = original.clone();
    expected.name = "Portal".to_owned();
    let model = loaded_model(
        vec![original],
        None,
        FakeValidator::accepting([(path.clone(), path)]),
    );

    assert_eq!(model.settings.manual_games, [expected]);
    assert!(model.settings_warning.is_none());
    assert!(model.settings.enabled);

    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::from_path(temp.path().join("overcrow/settings.json"));
    model.save_selections(&store).unwrap();
    assert_eq!(
        store.load().settings.manual_games,
        model.settings.manual_games
    );
}

#[test]
fn loaded_exe_manual_games_are_dropped_and_merge_the_settings_warning() {
    let path = PathBuf::from("/games/portal.exe");
    let model = loaded_model(
        vec![loaded_manual_game("Portal", path.clone())],
        Some("existing settings warning"),
        FakeValidator::accepting([(path.clone(), path)]),
    );

    assert_one_loaded_game_was_dropped(&model);
    let warning = model.settings_warning.as_deref().unwrap();
    assert!(warning.contains("existing settings warning"));
    assert_eq!(warning.matches("Dropped").count(), 1);
}

#[test]
fn loaded_missing_manual_games_are_dropped() {
    let path = PathBuf::from("/games/missing");
    let model = loaded_model(
        vec![loaded_manual_game("Missing", path.clone())],
        None,
        FakeValidator::rejecting(
            path,
            SelectionError::ExecutableValidationFailed(io::ErrorKind::NotFound),
        ),
    );

    assert_one_loaded_game_was_dropped(&model);
}

#[test]
fn loaded_directory_manual_games_are_dropped() {
    let path = PathBuf::from("/games/directory");
    let model = loaded_model(
        vec![loaded_manual_game("Directory", path.clone())],
        None,
        FakeValidator::rejecting(path, SelectionError::ExecutableNotRegularFile),
    );

    assert_one_loaded_game_was_dropped(&model);
}

#[test]
fn loaded_non_executable_manual_games_are_dropped() {
    let path = PathBuf::from("/games/not-executable");
    let model = loaded_model(
        vec![loaded_manual_game("Not executable", path.clone())],
        None,
        FakeValidator::rejecting(path, SelectionError::ExecutableNotExecutable),
    );

    assert_one_loaded_game_was_dropped(&model);
}

#[test]
fn loaded_noncanonical_aliases_are_dropped_instead_of_silently_rewritten() {
    let alias = PathBuf::from("/games/alias");
    let canonical = PathBuf::from("/games/canonical");
    let mut entry = loaded_manual_game("Alias", alias.clone());
    entry.id = expected_manual_id(&canonical);
    let model = loaded_model(
        vec![entry],
        None,
        FakeValidator::accepting([(alias, canonical)]),
    );

    assert_one_loaded_game_was_dropped(&model);
}

#[test]
fn duplicate_loaded_canonical_paths_keep_only_the_first_entry() {
    let path = PathBuf::from("/games/portal");
    let first = loaded_manual_game("First", path.clone());
    let second = loaded_manual_game("Second", path.clone());
    let model = loaded_model(
        vec![first.clone(), second],
        None,
        FakeValidator::accepting([(path.clone(), path)]),
    );

    assert_eq!(model.settings.manual_games, [first]);
    assert!(
        model
            .settings_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("Dropped 1 invalid manual game selection"))
    );
}

#[test]
fn loaded_manual_games_with_wrong_stable_ids_are_dropped() {
    let path = PathBuf::from("/games/portal");
    let mut entry = loaded_manual_game("Portal", path.clone());
    entry.id = "local.wrong".to_owned();
    let model = loaded_model(
        vec![entry],
        None,
        FakeValidator::accepting([(path.clone(), path)]),
    );

    assert_one_loaded_game_was_dropped(&model);
}

#[test]
fn loaded_manual_games_with_empty_names_are_dropped() {
    let path = PathBuf::from("/games/portal");
    let model = loaded_model(
        vec![loaded_manual_game("  \t", path.clone())],
        None,
        FakeValidator::accepting([(path.clone(), path)]),
    );

    assert_one_loaded_game_was_dropped(&model);
}

#[test]
fn loaded_non_utf8_manual_games_are_dropped_before_save() {
    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![
        b'/', b'g', b'a', b'm', b'e', b's', b'/', 0xff,
    ]));
    let model = loaded_model(
        vec![loaded_manual_game("Non UTF-8", path.clone())],
        None,
        FakeValidator::accepting([(path.clone(), path)]),
    );

    assert_one_loaded_game_was_dropped(&model);
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::from_path(temp.path().join("overcrow/settings.json"));
    model.save_selections(&store).unwrap();
}

#[test]
fn saving_rejects_publicly_injected_invalid_manual_settings() {
    let path = PathBuf::from("/games/portal.exe");
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([(path.clone(), path.clone())]),
    );
    model
        .settings
        .manual_games
        .push(loaded_manual_game("Portal", path));
    let temp = tempfile::tempdir().unwrap();
    let settings_path = temp.path().join("overcrow/settings.json");
    let store = SettingsStore::from_path(&settings_path);

    let error = model.save_selections(&store).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("manual game selections"));
    assert!(!settings_path.exists());
}

#[test]
fn saving_rejects_a_manual_executable_that_disappeared_and_preserves_prior_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let settings_path = temp.path().join("overcrow/settings.json");
    let store = SettingsStore::from_path(&settings_path);
    let prior = LifecycleSettings {
        selected_steam_app_ids: BTreeSet::from([620]),
        ..LifecycleSettings::default()
    };
    store.save(&prior).unwrap();
    let prior_bytes = fs::read(&settings_path).unwrap();

    let (_executable_temp, executable) = native_validator_fixture(0o700);
    let model = loaded_model(
        vec![loaded_manual_game("Portal", executable.clone())],
        None,
        NativePathValidator,
    );
    fs::remove_file(executable).unwrap();

    let error = model.save_selections(&store).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read(settings_path).unwrap(), prior_bytes);
}

#[test]
fn saving_rejects_a_publicly_edited_whitespace_name_without_trimming_it() {
    let requested = PathBuf::from("/chosen/portal");
    let canonical = PathBuf::from("/games/portal");
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([(requested.clone(), canonical)]),
    );
    model.add_manual_game("Portal", &requested).unwrap();
    model.settings.manual_games[0].name = "  Portal  ".to_owned();
    let temp = tempfile::tempdir().unwrap();
    let settings_path = temp.path().join("overcrow/settings.json");
    let store = SettingsStore::from_path(&settings_path);

    let error = model.save_selections(&store).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(model.settings.manual_games[0].name, "  Portal  ");
    assert!(!settings_path.exists());
}

#[test]
fn manual_games_reject_relative_paths_without_calling_the_validator() {
    let requested = PathBuf::from("portal");
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([(requested.clone(), PathBuf::from("/games/portal"))]),
    );

    let error = model.add_manual_game("Portal", &requested).unwrap_err();

    assert_eq!(error, SelectionError::ExecutableNotAbsolute);
    assert!(model.settings.manual_games.is_empty());
}

#[test]
fn manual_games_reject_empty_and_whitespace_only_names() {
    for name in ["", "  \t\n"] {
        let requested = PathBuf::from("/chosen/portal");
        let mut model = model_with(
            LifecycleSettings::default(),
            Vec::new(),
            FakeValidator::accepting([(requested.clone(), PathBuf::from("/games/portal"))]),
        );

        let error = model.add_manual_game(name, &requested).unwrap_err();

        assert_eq!(error, SelectionError::EmptyName);
        assert!(model.settings.manual_games.is_empty());
    }
}

#[test]
fn manual_game_names_are_trimmed_before_storage() {
    let requested = PathBuf::from("/chosen/portal");
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([(requested.clone(), PathBuf::from("/games/portal"))]),
    );

    let id = model.add_manual_game("  Portal  ", &requested).unwrap();

    assert_eq!(model.settings.manual_games[0].name, "Portal");
    assert_eq!(model.settings.manual_games[0].id, id);
}

#[test]
fn duplicate_manual_canonical_paths_are_rejected_without_mutation() {
    let first = PathBuf::from("/chosen/portal");
    let alias = PathBuf::from("/chosen/portal-alias");
    let canonical = PathBuf::from("/games/portal");
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([
            (first.clone(), canonical.clone()),
            (alias.clone(), canonical),
        ]),
    );
    model.add_manual_game("Portal", &first).unwrap();

    let error = model.add_manual_game("Portal alias", &alias).unwrap_err();

    assert_eq!(error, SelectionError::DuplicateManualExecutable);
    assert_eq!(model.settings.manual_games.len(), 1);
}

#[test]
fn duplicate_manual_display_names_are_allowed_for_distinct_identities() {
    let first = PathBuf::from("/chosen/first");
    let second = PathBuf::from("/chosen/second");
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([
            (first.clone(), PathBuf::from("/games/first")),
            (second.clone(), PathBuf::from("/games/second")),
        ]),
    );

    let first_id = model.add_manual_game("Same name", &first).unwrap();
    let second_id = model.add_manual_game("Same name", &second).unwrap();

    assert_ne!(first_id, second_id);
    assert_eq!(
        model
            .settings
            .manual_games
            .iter()
            .map(|game| game.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Same name", "Same name"]
    );
}

#[test]
fn validator_errors_are_propagated_without_mutating_manual_games() {
    let requested = PathBuf::from("/chosen/missing");
    let expected = SelectionError::ExecutableValidationFailed(io::ErrorKind::PermissionDenied);
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::rejecting(requested.clone(), expected.clone()),
    );

    let actual = model.add_manual_game("Missing", &requested).unwrap_err();

    assert_eq!(actual, expected);
    assert!(model.settings.manual_games.is_empty());
}

#[test]
fn manual_ids_use_a_stable_sha256_prefix_of_the_canonical_path_bytes() {
    let requested = PathBuf::from("/chosen/portal");
    let canonical = PathBuf::from("/games/portal2");
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([(requested.clone(), canonical)]),
    );

    let first = model.add_manual_game("Portal", &requested).unwrap();
    assert_eq!(first, "local.ed447fe81e8a95618ecba32e9ca7366b");

    model.remove_manual_game(&first);
    let second = model
        .add_manual_game("A renamed Portal", &requested)
        .unwrap();
    assert_eq!(second, first);
}

#[test]
fn non_utf8_canonical_paths_are_rejected_without_breaking_future_saves() {
    let requested = PathBuf::from("/chosen/non-utf8");
    let canonical = PathBuf::from(std::ffi::OsString::from_vec(vec![
        b'/', b'g', b'a', b'm', b'e', b's', b'/', 0xff,
    ]));
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([(requested.clone(), canonical)]),
    );
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::from_path(temp.path().join("overcrow/settings.json"));

    let error = model.add_manual_game("Non UTF-8", &requested).unwrap_err();

    assert_eq!(error, SelectionError::ExecutableNotUtf8);
    assert!(model.settings.manual_games.is_empty());
    model.save_selections(&store).unwrap();
    assert!(store.load().settings.manual_games.is_empty());
}

#[test]
fn an_occupied_stable_id_fails_deterministically_without_rewriting_entries() {
    let requested = PathBuf::from("/chosen/portal");
    let canonical = PathBuf::from("/games/portal2");
    let occupied = ManualGame {
        id: "local.ed447fe81e8a95618ecba32e9ca7366b".to_owned(),
        name: "Existing".to_owned(),
        executable: PathBuf::from("/games/different"),
    };
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([(requested.clone(), canonical)]),
    );
    // Inject an artificial 128-bit collision after load hardening so this test reaches the
    // add-time collision guard; naturally producing such a prefix collision is infeasible.
    model.settings.manual_games.push(occupied.clone());

    let error = model.add_manual_game("Portal", &requested).unwrap_err();

    assert_eq!(error, SelectionError::ManualGameIdCollision);
    assert_eq!(model.settings.manual_games, [occupied]);
}

#[test]
fn manual_games_are_removed_by_stable_id() {
    let requested = PathBuf::from("/chosen/portal");
    let mut model = model_with(
        LifecycleSettings::default(),
        Vec::new(),
        FakeValidator::accepting([(requested.clone(), PathBuf::from("/games/portal"))]),
    );
    let id = model.add_manual_game("Portal", &requested).unwrap();

    assert!(model.remove_manual_game(&id));
    assert!(model.settings.manual_games.is_empty());
    assert!(!model.remove_manual_game(&id));
}

fn native_validator_fixture(mode: u32) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("game");
    fs::write(&executable, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(mode)).unwrap();
    (temp, executable)
}

#[test]
fn native_validator_canonicalizes_symlinks_to_regular_executable_files() {
    let (temp, executable) = native_validator_fixture(0o100);
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&executable, &alias).unwrap();

    let canonical = NativePathValidator.canonical_executable(&alias).unwrap();

    assert_eq!(canonical, executable.canonicalize().unwrap());
}

#[test]
fn native_validator_rejects_missing_paths() {
    let temp = tempfile::tempdir().unwrap();
    let error = NativePathValidator
        .canonical_executable(&temp.path().join("missing"))
        .unwrap_err();

    assert_eq!(
        error,
        SelectionError::ExecutableValidationFailed(io::ErrorKind::NotFound)
    );
}

#[test]
fn native_validator_rejects_non_files() {
    let temp = tempfile::tempdir().unwrap();
    let error = NativePathValidator
        .canonical_executable(temp.path())
        .unwrap_err();

    assert_eq!(error, SelectionError::ExecutableNotRegularFile);
}

#[test]
fn native_validator_rejects_files_without_any_unix_execute_bit() {
    let (_temp, executable) = native_validator_fixture(0o600);

    let error = NativePathValidator
        .canonical_executable(&executable)
        .unwrap_err();

    assert_eq!(error, SelectionError::ExecutableNotExecutable);
}

#[test]
fn native_validator_accepts_each_class_of_unix_execute_bit() {
    for mode in [0o100, 0o010, 0o001] {
        let (_temp, executable) = native_validator_fixture(mode);
        assert!(
            NativePathValidator
                .canonical_executable(&executable)
                .is_ok(),
            "mode {mode:o} should be executable"
        );
    }
}

#[test]
fn native_validator_fails_closed_for_exe_paths_case_insensitively() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["game.exe", "GAME.EXE"] {
        let path = temp.path().join(name);
        fs::write(&path, b"MZ").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();

        let error = NativePathValidator.canonical_executable(&path).unwrap_err();

        assert_eq!(error, SelectionError::WineIdentityUnavailable);
    }
}

#[test]
fn a_native_alias_resolving_to_an_exe_also_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("game.exe");
    let alias = temp.path().join("game");
    fs::write(&executable, b"MZ").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&executable, &alias).unwrap();

    let error = NativePathValidator
        .canonical_executable(&alias)
        .unwrap_err();

    assert_eq!(error, SelectionError::WineIdentityUnavailable);
}
