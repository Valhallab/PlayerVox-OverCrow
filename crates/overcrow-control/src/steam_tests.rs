use std::{
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
    path::Path,
};

use crate::{
    MAX_KEYVALUES_NESTING_DEPTH, MAX_LIBRARY_VDF_BYTES, MAX_MANIFEST_BYTES,
    MAX_MANIFESTS_INSPECTED, MAX_SECONDARY_LIBRARIES, MAX_WARNINGS, candidate_steam_roots,
    discover_from_roots,
};

fn materialize_fixture(temp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = temp.join("root");
    let secondary = temp.join("secondary");
    fs::create_dir_all(root.join("steamapps/common/Portal 2")).unwrap();
    fs::create_dir_all(root.join("steam/games")).unwrap();
    fs::create_dir_all(secondary.join("steamapps/common/Palworld")).unwrap();

    let libraryfolders = include_str!("../tests/fixtures/steam/root/steamapps/libraryfolders.vdf")
        .replace("__ROOT_LIBRARY__", &root.to_string_lossy())
        .replace("__SECONDARY_LIBRARY__", &secondary.to_string_lossy());
    fs::write(root.join("steamapps/libraryfolders.vdf"), libraryfolders).unwrap();
    fs::write(
        root.join("steamapps/appmanifest_620.acf"),
        include_bytes!("../tests/fixtures/steam/root/steamapps/appmanifest_620.acf"),
    )
    .unwrap();
    fs::write(
        secondary.join("steamapps/appmanifest_1623730.acf"),
        include_bytes!("../tests/fixtures/steam/secondary/steamapps/appmanifest_1623730.acf"),
    )
    .unwrap();
    fs::write(root.join("steam/games/portal2-icon.ico"), b"local icon").unwrap();

    (root, secondary)
}

fn write_root_only_library_list(root: &Path) {
    fs::write(
        root.join("steamapps/libraryfolders.vdf"),
        format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
            root.display()
        ),
    )
    .unwrap();
}

fn write_malformed_manifest_names(steamapps: &Path) {
    for index in 0..=MAX_MANIFESTS_INSPECTED {
        fs::write(
            steamapps.join(format!("appmanifest_000-bad-{index:03}.acf")),
            b"malformed candidate",
        )
        .unwrap();
    }
}

#[test]
fn candidate_roots_are_canonical_and_deduplicate_aliases() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let standard = home.join(".local/share/Steam");
    let legacy = home.join(".steam/steam");
    let flatpak = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
    fs::create_dir_all(&standard).unwrap();
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    symlink(&standard, &legacy).unwrap();
    fs::create_dir_all(&flatpak).unwrap();
    fs::create_dir_all(home.join(".var/app/com.valvesoftware.Steam/.local/share")).unwrap();

    let roots = candidate_steam_roots(home);

    assert_eq!(
        roots,
        vec![
            standard.canonicalize().unwrap(),
            flatpak.canonicalize().unwrap()
        ]
    );
    assert!(roots.iter().all(|path| path.is_dir() && path.is_absolute()));
}

#[test]
fn candidate_roots_follow_standard_legacy_and_flatpak_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let candidates = [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    for candidate in &candidates {
        fs::create_dir_all(candidate).unwrap();
    }

    assert_eq!(
        candidate_steam_roots(home),
        candidates
            .into_iter()
            .map(|path| path.canonicalize().unwrap())
            .collect::<Vec<_>>()
    );
}

#[test]
fn candidate_roots_ignore_missing_paths_and_regular_files() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let legacy = home.join(".steam/steam");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, b"not a directory").unwrap();

    assert!(candidate_steam_roots(home).is_empty());
}

#[test]
fn discovers_games_and_local_metadata_across_keyvalues_v1_libraries() {
    let temp = tempfile::tempdir().unwrap();
    let (root, secondary) = materialize_fixture(temp.path());

    let report = discover_from_roots(std::slice::from_ref(&root));

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| (game.app_id, game.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(620, "Portal 2"), (1_623_730, "Palworld")]
    );
    assert_eq!(
        report.games[0].install_dir,
        root.join("steamapps/common/Portal 2")
    );
    assert_eq!(
        report.games[0].icon,
        Some(root.join("steam/games/portal2-icon.ico"))
    );
    assert_eq!(
        report.games[1].install_dir,
        secondary.join("steamapps/common/Palworld")
    );
    assert_eq!(report.games[1].icon, None);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn legacy_direct_keyvalues_library_paths_are_supported() {
    let temp = tempfile::tempdir().unwrap();
    let (root, secondary) = materialize_fixture(temp.path());
    fs::write(
        root.join("steamapps/libraryfolders.vdf"),
        format!(
            "\"LibraryFolders\"\n{{\n    \"1\"    \"{}\"\n}}\n",
            secondary.display()
        ),
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![620, 1_623_730]
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn zero_app_ids_are_ignored_only_when_the_filename_identity_is_also_zero() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    fs::write(
        root.join("steamapps/appmanifest_0.acf"),
        b"\"AppState\" { \"appid\" \"0\" }",
    )
    .unwrap();
    let mismatched = root.join("steamapps/appmanifest_999.acf");
    fs::write(
        &mismatched,
        b"\"AppState\" { \"appid\" \"0\" \"name\" \"Zero\" \"installdir\" \"Zero\" }",
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 2);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&mismatched.display().to_string()));
    assert!(report.warnings[0].contains("does not match filename"));
}

#[test]
fn a_zero_filename_is_parsed_and_must_match_the_manifest_identity() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let mismatched = root.join("steamapps/appmanifest_0.acf");
    fs::write(
        &mismatched,
        b"\"AppState\" { \"appid\" \"620\" \"name\" \"Wrong\" \"installdir\" \"Portal 2\" }",
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 2);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&mismatched.display().to_string()));
    assert!(report.warnings[0].contains("does not match filename"));
}

#[test]
fn malformed_appmanifest_like_names_warn_while_unrelated_files_are_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let steamapps = root.join("steamapps");
    let malformed = steamapps.join("appmanifest_not-an-id.acf");
    let non_utf8 = steamapps.join(OsString::from_vec(b"appmanifest_\xff.acf".to_vec()));
    fs::write(&malformed, b"ignored contents").unwrap();
    fs::write(&non_utf8, b"ignored contents").unwrap();
    fs::write(steamapps.join("notes.acf"), b"unrelated").unwrap();
    fs::write(steamapps.join("appmanifest_42.txt"), b"unrelated").unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 2);
    assert_eq!(report.warnings.len(), 2);
    assert!(report.warnings.iter().any(|warning| {
        warning.starts_with(&malformed.display().to_string())
            && warning.contains("malformed app manifest filename")
    }));
    assert!(report.warnings.iter().any(|warning| {
        warning.starts_with(&non_utf8.display().to_string())
            && warning.contains("malformed app manifest filename")
    }));
}

#[test]
fn a_malformed_secondary_manifest_does_not_hide_valid_root_games() {
    let temp = tempfile::tempdir().unwrap();
    let (root, secondary) = materialize_fixture(temp.path());
    let malformed = secondary.join("steamapps/appmanifest_1623730.acf");
    fs::write(&malformed, b"\"AppState\" { this is not valid").unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![620]
    );
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&malformed.display().to_string()));
}

#[test]
fn a_malformed_library_entry_does_not_hide_other_libraries() {
    let temp = tempfile::tempdir().unwrap();
    let (root, secondary) = materialize_fixture(temp.path());
    let library_file = root.join("steamapps/libraryfolders.vdf");
    fs::write(
        &library_file,
        format!(
            concat!(
                "\"libraryfolders\"\n{{\n",
                "    \"1\" {{ \"label\" \"missing path\" }}\n",
                "    \"2\" {{ \"path\" \"{}\" }}\n",
                "}}\n"
            ),
            secondary.display()
        ),
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![620, 1_623_730]
    );
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&library_file.display().to_string()));
    assert!(report.warnings[0].contains("library entry 1"));
}

#[test]
fn duplicate_app_ids_keep_the_first_library_in_root_precedence_order() {
    let temp = tempfile::tempdir().unwrap();
    let (root, secondary) = materialize_fixture(temp.path());
    fs::remove_file(secondary.join("steamapps/appmanifest_1623730.acf")).unwrap();
    fs::create_dir(secondary.join("steamapps/common/Duplicate")).unwrap();
    fs::write(
        secondary.join("steamapps/appmanifest_620.acf"),
        b"\"AppState\" { \"appid\" \"620\" \"name\" \"Duplicate\" \"installdir\" \"Duplicate\" }",
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 1);
    assert_eq!(report.games[0].app_id, 620);
    assert_eq!(report.games[0].name, "Portal 2");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn stale_manifest_with_a_missing_install_directory_is_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    fs::remove_dir(root.join("steamapps/common/Portal 2")).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![1_623_730]
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn hostile_manifest_fields_fail_closed_without_hiding_valid_games() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let steamapps = root.join("steamapps");
    let invalid_manifests = [
        (
            "appmanifest_1.acf",
            "\"WrongRoot\" { \"appid\" \"1\" \"name\" \"Wrong\" \"installdir\" \"Wrong\" }",
        ),
        (
            "appmanifest_2.acf",
            "\"AppState\" { \"appid\" \"not-a-number\" \"name\" \"Bad ID\" \"installdir\" \"Bad\" }",
        ),
        (
            "appmanifest_3.acf",
            "\"AppState\" { \"appid\" \"3\" \"name\" \"Traversal\" \"installdir\" \"../escape\" }",
        ),
        (
            "appmanifest_4.acf",
            "\"AppState\" { \"appid\" \"4\" \"name\" \"\" \"installdir\" \"Empty\" }",
        ),
        (
            "appmanifest_5.acf",
            "\"AppState\" { \"appid\" \"5\" \"appid\" \"5\" \"name\" \"Duplicate key\" \"installdir\" \"Duplicate\" }",
        ),
    ];
    for (filename, contents) in invalid_manifests {
        fs::write(steamapps.join(filename), contents).unwrap();
    }

    let result = std::panic::catch_unwind(|| discover_from_roots(&[root]));
    let report = result.expect("discovery must not panic for untrusted manifests");

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![620, 1_623_730]
    );
    assert_eq!(report.warnings.len(), 5);
    assert!(
        report
            .warnings
            .iter()
            .all(|warning| warning.starts_with(&steamapps.display().to_string()))
    );
}

#[test]
fn oversized_manifest_names_are_rejected_before_storage() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let manifest = root.join("steamapps/appmanifest_620.acf");
    fs::write(
        &manifest,
        format!(
            "\"AppState\" {{ \"appid\" \"620\" \"name\" \"{}\" \"installdir\" \"Portal 2\" }}",
            "x".repeat(crate::MAX_CONTROL_GAME_NAME_BYTES + 1)
        ),
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![1_623_730]
    );
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("name exceeds byte limit"));
}

#[test]
fn every_retained_discovery_warning_has_a_fixed_byte_bound() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let library_file = root.join("steamapps/libraryfolders.vdf");
    fs::write(
        &library_file,
        format!(
            "\"libraryfolders\" {{ \"1\" {{ \"path\" \"/{}\" }} }}",
            "x".repeat(crate::presentation::MAX_CONTROL_MESSAGE_BYTES * 2)
        ),
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert!(!report.warnings.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .all(|warning| warning.len() <= crate::presentation::MAX_CONTROL_MESSAGE_BYTES)
    );
}

#[test]
fn the_root_library_is_scanned_even_when_its_library_list_is_malformed() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let library_file = root.join("steamapps/libraryfolders.vdf");
    fs::write(&library_file, b"not valid KeyValues").unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![620]
    );
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&library_file.display().to_string()));
}

#[test]
fn oversized_library_lists_are_rejected_before_keyvalues_parsing() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let library_file = root.join("steamapps/libraryfolders.vdf");
    fs::write(&library_file, vec![b' '; MAX_LIBRARY_VDF_BYTES + 1]).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![620]
    );
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("exceeds byte limit"));
}

#[test]
fn oversized_manifests_are_rejected_before_keyvalues_parsing() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let manifest = root.join("steamapps/appmanifest_620.acf");
    let mut contents =
        b"\"AppState\" { \"appid\" \"620\" \"name\" \"Portal 2\" \"installdir\" \"Portal 2\" }"
            .to_vec();
    contents.resize(MAX_MANIFEST_BYTES + 1, b' ');
    fs::write(&manifest, contents).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![1_623_730]
    );
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&manifest.display().to_string()));
    assert!(report.warnings[0].contains("exceeds byte limit"));
}

#[test]
fn excessive_keyvalues_nesting_is_rejected_before_parser_recursion() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let manifest = root.join("steamapps/appmanifest_620.acf");
    let mut contents = String::from(
        "\"AppState\" { \"appid\" \"620\" \"name\" \"Portal 2\" \"installdir\" \"Portal 2\" ",
    );
    for index in 0..MAX_KEYVALUES_NESTING_DEPTH {
        contents.push_str(&format!("\"level{index}\" {{ "));
    }
    contents.push_str(&"} ".repeat(MAX_KEYVALUES_NESTING_DEPTH + 1));
    fs::write(&manifest, contents).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![1_623_730]
    );
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("nesting depth limit"));
}

#[test]
fn keyvalues_preflight_ignores_braces_in_escaped_strings_and_comments() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    fs::write(
        root.join("steamapps/appmanifest_620.acf"),
        concat!(
            "// ignored braces { } } {\n",
            "\"AppState\"\n{\n",
            "  \"appid\" \"620\"\n",
            "  \"name\" \"Portal \\\"Quoted\\\" {Edition}\"\n",
            "  \"installdir\" \"Portal 2\" // ignored } } }\n",
            "  \"icon\" \"portal2-icon\"\n",
            "}\n"
        ),
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games[0].app_id, 620);
    assert_eq!(report.games[0].name, "Portal \"Quoted\" {Edition}");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn unbalanced_keyvalues_braces_are_rejected_by_preflight() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let manifest = root.join("steamapps/appmanifest_620.acf");
    fs::write(
        &manifest,
        b"\"AppState\" { \"appid\" \"620\" \"name\" \"Portal 2\" \"installdir\" \"Portal 2\"",
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 1);
    assert_eq!(report.games[0].app_id, 1_623_730);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("unbalanced KeyValues braces"));
}

#[test]
fn secondary_library_count_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let mut libraryfolders = String::from("\"libraryfolders\" {\n");
    for index in 0..=MAX_SECONDARY_LIBRARIES {
        let library = temp.path().join(format!("library-{index:03}"));
        fs::create_dir_all(library.join("steamapps")).unwrap();
        libraryfolders.push_str(&format!(
            "\"{}\" {{ \"path\" \"{}\" }}\n",
            index + 1,
            library.display()
        ));
    }
    libraryfolders.push_str("}\n");
    fs::write(root.join("steamapps/libraryfolders.vdf"), libraryfolders).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 1);
    assert_eq!(report.games[0].app_id, 620);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("secondary library limit"))
    );
}

#[test]
fn manifest_inspection_count_is_bounded_deterministically() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    write_root_only_library_list(&root);
    fs::remove_file(root.join("steamapps/appmanifest_620.acf")).unwrap();
    for app_id in 1..=(MAX_MANIFESTS_INSPECTED + 1) {
        fs::write(
            root.join(format!("steamapps/appmanifest_{app_id}.acf")),
            format!(
                "\"AppState\" {{ \"appid\" \"{app_id}\" \"name\" \"Game {app_id}\" \"installdir\" \"Portal 2\" }}"
            ),
        )
        .unwrap();
    }

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), MAX_MANIFESTS_INSPECTED);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("manifest inspection limit"))
    );
    assert!(
        report
            .games
            .windows(2)
            .all(|pair| pair[0].app_id < pair[1].app_id)
    );
}

#[test]
fn retained_warning_count_is_bounded_with_a_truncation_notice() {
    let temp = tempfile::tempdir().unwrap();
    let roots = (0..=MAX_WARNINGS)
        .map(|index| temp.path().join(format!("missing-root-{index:03}")))
        .collect::<Vec<_>>();

    let report = discover_from_roots(&roots);

    assert_eq!(report.warnings.len(), MAX_WARNINGS);
    assert!(report.warnings.last().unwrap().contains("warning limit"));
}

#[test]
fn a_steamapps_symlink_must_not_escape_its_canonical_library_root() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let steamapps = root.join("steamapps");
    let outside = temp.path().join("outside-steamapps");
    fs::rename(&steamapps, &outside).unwrap();
    symlink(&outside, &steamapps).unwrap();

    let report = discover_from_roots(&[root]);

    assert!(report.games.is_empty());
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&steamapps.display().to_string()));
    assert!(report.warnings[0].contains("outside canonical library root"));
}

#[test]
fn a_manifest_symlink_must_not_escape_canonical_steamapps() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let manifest = root.join("steamapps/appmanifest_620.acf");
    let outside = temp.path().join("outside-manifest.acf");
    fs::rename(&manifest, &outside).unwrap();
    symlink(&outside, &manifest).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 1);
    assert_eq!(report.games[0].app_id, 1_623_730);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&manifest.display().to_string()));
    assert!(report.warnings[0].contains("outside canonical steamapps"));
}

#[test]
fn an_install_directory_symlink_must_not_escape_canonical_common() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let install_dir = root.join("steamapps/common/Portal 2");
    let outside = temp.path().join("outside-install");
    fs::create_dir(&outside).unwrap();
    fs::remove_dir(&install_dir).unwrap();
    symlink(&outside, &install_dir).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 1);
    assert_eq!(report.games[0].app_id, 1_623_730);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&install_dir.display().to_string()));
    assert!(report.warnings[0].contains("outside canonical common directory"));
}

#[test]
fn an_icon_symlink_must_not_escape_canonical_steam_games() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let icon = root.join("steam/games/portal2-icon.ico");
    let outside = temp.path().join("outside-icon.ico");
    fs::write(&outside, b"external icon").unwrap();
    fs::remove_file(&icon).unwrap();
    symlink(&outside, &icon).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 2);
    assert_eq!(report.games[0].app_id, 620);
    assert_eq!(report.games[0].icon, None);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].starts_with(&icon.display().to_string()));
    assert!(report.warnings[0].contains("outside canonical Steam games directory"));
}

#[test]
fn slash_star_markers_do_not_hide_excessive_keyvalues_nesting() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let manifest = root.join("steamapps/appmanifest_620.acf");
    let mut contents = String::from(
        "\"AppState\" { \"appid\" \"620\" \"name\" \"Portal 2\" \"installdir\" \"Portal 2\" /* ",
    );
    for index in 0..MAX_KEYVALUES_NESTING_DEPTH {
        contents.push_str(&format!("\"level{index}\" {{ "));
    }
    contents.push_str(&"} ".repeat(MAX_KEYVALUES_NESTING_DEPTH));
    contents.push_str(" */ }");
    fs::write(&manifest, contents).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 1);
    assert_eq!(report.games[0].app_id, 1_623_730);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("nesting depth limit"));
}

#[test]
fn malformed_names_do_not_displace_a_valid_root_manifest_or_consume_quota() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    write_malformed_manifest_names(&root.join("steamapps"));

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![620, 1_623_730]
    );
    assert_eq!(report.warnings.len(), MAX_WARNINGS);
    assert!(report.warnings.last().unwrap().contains("warning limit"));
}

#[test]
fn malformed_names_in_a_later_library_do_not_exhaust_global_parse_quota() {
    let temp = tempfile::tempdir().unwrap();
    let (root, secondary) = materialize_fixture(temp.path());
    write_malformed_manifest_names(&secondary.join("steamapps"));

    let report = discover_from_roots(&[root]);

    assert_eq!(
        report
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![620, 1_623_730]
    );
    assert_eq!(report.warnings.len(), MAX_WARNINGS);
    assert!(report.warnings.last().unwrap().contains("warning limit"));
}

#[test]
fn slashes_inside_an_unquoted_token_do_not_hide_excessive_nesting() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let manifest = root.join("steamapps/appmanifest_620.acf");
    let mut contents = String::from(
        "\"AppState\" {\n\"appid\" \"620\"\n\"name\" \"Portal 2\"\n\"installdir\" \"Portal 2\"\nmarker// { ",
    );
    for index in 0..MAX_KEYVALUES_NESTING_DEPTH {
        contents.push_str(&format!("\"level{index}\" {{ "));
    }
    contents.push_str(&"} ".repeat(MAX_KEYVALUES_NESTING_DEPTH + 1));
    contents.push_str("\n}\n");
    fs::write(&manifest, contents).unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games.len(), 1);
    assert_eq!(report.games[0].app_id, 1_623_730);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("nesting depth limit"));
}

#[test]
fn slashes_between_tokens_still_start_a_line_comment() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _) = materialize_fixture(temp.path());
    let manifest = root.join("steamapps/appmanifest_620.acf");
    let hidden_braces = "{ ".repeat(MAX_KEYVALUES_NESTING_DEPTH + 1);
    fs::write(
        &manifest,
        format!(
            concat!(
                "\"AppState\" {{\n",
                "\"appid\" \"620\"\n",
                "\"name\" \"Portal 2\"\n",
                "\"installdir\" \"Portal 2\"\n",
                "// {}\n",
                "}}\n"
            ),
            hidden_braces
        ),
    )
    .unwrap();

    let report = discover_from_roots(&[root]);

    assert_eq!(report.games[0].app_id, 620);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}
