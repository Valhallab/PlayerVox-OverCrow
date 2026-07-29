use std::{fs, os::unix::fs::symlink, path::Path};

use crate::steam_binary::{
    MAX_APPINFO_BYTES, MAX_BINARY_VDF_DEPTH, SteamAppType, SteamShortcut, read_binary_metadata,
};

const APPINFO_MAGIC_V41: u32 = 0x0756_4429;

fn object(bytes: &mut Vec<u8>, key: &str) {
    bytes.push(0);
    cstring(bytes, key);
}

fn indexed_object(bytes: &mut Vec<u8>, key_index: u32) {
    bytes.push(0);
    bytes.extend_from_slice(&key_index.to_le_bytes());
}

fn string(bytes: &mut Vec<u8>, key: &str, value: &str) {
    bytes.push(1);
    cstring(bytes, key);
    cstring(bytes, value);
}

fn indexed_string(bytes: &mut Vec<u8>, key_index: u32, value: &str) {
    bytes.push(1);
    bytes.extend_from_slice(&key_index.to_le_bytes());
    cstring(bytes, value);
}

fn int32(bytes: &mut Vec<u8>, key: &str, value: u32) {
    bytes.push(2);
    cstring(bytes, key);
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn cstring(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
}

fn end(bytes: &mut Vec<u8>) {
    bytes.push(8);
}

fn shortcut_entry(app_id: u32, name: &str) -> Vec<u8> {
    shortcut_entry_bytes(app_id, name.as_bytes())
}

fn shortcut_entry_bytes(app_id: u32, name: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    object(&mut bytes, "shortcuts");
    object(&mut bytes, "0");
    int32(&mut bytes, "appid", app_id);
    bytes.push(1);
    cstring(&mut bytes, "appname");
    bytes.extend_from_slice(name);
    bytes.push(0);
    string(&mut bytes, "exe", "\"/private/game.exe\"");
    string(&mut bytes, "LaunchOptions", "--private-value");
    end(&mut bytes);
    end(&mut bytes);
    bytes
}

fn appinfo_v41(entries: &[(u32, &str)]) -> Vec<u8> {
    let string_table = ["appinfo", "common", "type"];
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&APPINFO_MAGIC_V41.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());

    for (app_id, app_type) in entries {
        let mut vdf = Vec::new();
        indexed_object(&mut vdf, 0);
        indexed_object(&mut vdf, 1);
        indexed_string(&mut vdf, 2, app_type);
        end(&mut vdf);
        end(&mut vdf);
        end(&mut vdf);

        bytes.extend_from_slice(&app_id.to_le_bytes());
        bytes.extend_from_slice(&(60_u32 + u32::try_from(vdf.len()).unwrap()).to_le_bytes());
        bytes.extend_from_slice(&[0; 60]);
        bytes.extend_from_slice(&vdf);
    }
    bytes.extend_from_slice(&0_u32.to_le_bytes());

    let string_table_offset = u64::try_from(bytes.len()).unwrap();
    bytes[8..16].copy_from_slice(&string_table_offset.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(string_table.len()).unwrap().to_le_bytes());
    for value in string_table {
        cstring(&mut bytes, value);
    }
    bytes
}

fn write_root_files(root: &Path, appinfo: &[u8], shortcuts: &[u8]) {
    fs::create_dir_all(root.join("appcache")).unwrap();
    fs::create_dir_all(root.join("userdata/25170504/config")).unwrap();
    fs::write(root.join("appcache/appinfo.vdf"), appinfo).unwrap();
    fs::write(
        root.join("userdata/25170504/config/shortcuts.vdf"),
        shortcuts,
    )
    .unwrap();
}

#[test]
fn reads_shortcut_identity_without_retaining_launch_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    write_root_files(
        &root,
        &appinfo_v41(&[]),
        &shortcut_entry(2_369_324_441, "Soulframe"),
    );

    let report = read_binary_metadata(&root);

    assert_eq!(
        report.shortcuts,
        [SteamShortcut {
            app_id: 2_369_324_441,
            name: "Soulframe".to_owned(),
        }]
    );
    assert!(
        format!("{report:?}").find("private").is_none(),
        "private shortcut metadata must not be retained"
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn reads_game_and_non_game_types_from_v41_appinfo() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    write_root_files(
        &root,
        &appinfo_v41(&[(10, "Game"), (20, "Tool")]),
        &shortcut_entry(42, "Shortcut"),
    );

    let report = read_binary_metadata(&root);

    assert!(report.warnings.is_empty(), "{report:?}");
    assert_eq!(report.app_types[&10], SteamAppType::Game);
    assert_eq!(report.app_types[&20], SteamAppType::NonGame);
}

#[test]
fn malformed_appinfo_does_not_hide_valid_shortcuts_or_panic() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    let mut malformed = Vec::new();
    malformed.extend_from_slice(&APPINFO_MAGIC_V41.to_le_bytes());
    malformed.extend_from_slice(&1_u32.to_le_bytes());
    malformed.extend_from_slice(&20_u64.to_le_bytes());
    malformed.extend_from_slice(&10_u32.to_le_bytes());
    malformed.extend_from_slice(&1_u32.to_le_bytes());
    malformed.extend_from_slice(&0_u32.to_le_bytes());
    write_root_files(&root, &malformed, &shortcut_entry(42, "Still valid"));

    let report = std::panic::catch_unwind(|| read_binary_metadata(&root))
        .expect("malformed local Steam metadata must never panic");

    assert_eq!(report.shortcuts[0].name, "Still valid");
    assert!(report.app_types.is_empty());
    assert_eq!(report.warnings.len(), 1);
}

#[test]
fn excessive_binary_depth_is_rejected_before_stack_recursion() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    let mut shortcuts = Vec::new();
    for index in 0..=MAX_BINARY_VDF_DEPTH {
        object(&mut shortcuts, &format!("level-{index}"));
    }
    for _ in 0..=MAX_BINARY_VDF_DEPTH {
        end(&mut shortcuts);
    }
    write_root_files(&root, &appinfo_v41(&[]), &shortcuts);

    let report = read_binary_metadata(&root);

    assert!(report.shortcuts.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("depth limit"))
    );
}

#[test]
fn invalid_shortcut_names_are_skipped_with_one_bounded_warning() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    write_root_files(
        &root,
        &appinfo_v41(&[]),
        &shortcut_entry_bytes(42, b"invalid-\xff"),
    );

    let report = read_binary_metadata(&root);

    assert!(report.shortcuts.is_empty());
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("invalid shortcut entry"));
}

#[test]
fn oversized_and_escaping_metadata_files_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    let outside = temp.path().join("outside-shortcuts.vdf");
    fs::create_dir_all(root.join("appcache")).unwrap();
    fs::create_dir_all(root.join("userdata/1/config")).unwrap();
    let appinfo = fs::File::create(root.join("appcache/appinfo.vdf")).unwrap();
    appinfo
        .set_len(u64::try_from(MAX_APPINFO_BYTES + 1).unwrap())
        .unwrap();
    fs::write(&outside, shortcut_entry(42, "Outside")).unwrap();
    symlink(&outside, root.join("userdata/1/config/shortcuts.vdf")).unwrap();

    let report = read_binary_metadata(&root);

    assert!(report.app_types.is_empty());
    assert!(report.shortcuts.is_empty());
    assert_eq!(report.warnings.len(), 2);
    assert!(report.warnings.iter().all(|warning| warning.len() <= 512));
}
