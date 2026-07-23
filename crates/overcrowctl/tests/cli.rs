use std::{fs, os::unix::fs::PermissionsExt, process::Command};

fn private_log_directory(state: &tempfile::TempDir) -> std::path::PathBuf {
    let overcrow = state.path().join("overcrow");
    let logs = overcrow.join("logs");
    fs::create_dir_all(&logs).expect("create log directory");
    fs::set_permissions(&overcrow, fs::Permissions::from_mode(0o700))
        .expect("make OverCrow state private");
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).expect("make logs private");
    logs
}

fn private_log(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).expect("write fixture log");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("make fixture private");
}

#[test]
fn unknown_command_exits_with_code_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_overcrowctl"))
        .arg("launch")
        .output()
        .expect("overcrowctl should start");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn dbus_error_exits_with_code_one() {
    let missing_bus = format!(
        "unix:path=/tmp/overcrow-missing-session-bus-{}",
        std::process::id()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_overcrowctl"))
        .arg("status")
        .env("DBUS_SESSION_BUS_ADDRESS", missing_bus)
        .output()
        .expect("overcrowctl should start");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn logs_print_recent_events_without_a_session_bus() {
    let state = tempfile::tempdir().expect("create state directory");
    let logs = private_log_directory(&state);
    private_log(
        &logs.join("core.log"),
        concat!(
            "2026-07-20T10:00:00.000Z INFO core game_detected app_id=230410\n",
            "2026-07-20T10:00:02.000Z INFO core overlay_mode_changed mode=Interactive\n",
        ),
    );
    private_log(
        &logs.join("overlay.log"),
        "2026-07-20T10:00:01.000Z INFO overlay core_connected generation=1\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_overcrowctl"))
        .arg("logs")
        .env("XDG_STATE_HOME", state.path())
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/tmp/overcrow-definitely-missing-bus",
        )
        .output()
        .expect("overcrowctl should start");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        concat!(
            "2026-07-20T10:00:00.000Z INFO core game_detected app_id=230410\n",
            "2026-07-20T10:00:01.000Z INFO overlay core_connected generation=1\n",
            "2026-07-20T10:00:02.000Z INFO core overlay_mode_changed mode=Interactive\n",
        )
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn logs_are_bounded_to_two_thousand_lines() {
    let state = tempfile::tempdir().expect("create state directory");
    let logs = private_log_directory(&state);
    let mut contents = String::new();
    for index in 0..=2_000 {
        contents.push_str(&format!(
            "2026-07-20T10:00:00.000Z INFO core event_{index:04}\n"
        ));
    }
    private_log(&logs.join("core.log"), &contents);

    let output = Command::new(env!("CARGO_BIN_EXE_overcrowctl"))
        .arg("logs")
        .env("XDG_STATE_HOME", state.path())
        .output()
        .expect("overcrowctl should start");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");

    assert!(output.status.success());
    assert_eq!(stdout.lines().count(), 2_000);
    assert!(!stdout.contains("event_0000"));
    assert!(stdout.contains("event_0001"));
    assert!(stdout.contains("event_2000"));
}
