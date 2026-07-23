use std::{io, path::PathBuf};

use overcrow_config::{LifecycleSettings, SettingsLoad};

use crate::{
    CONTROL_LOG_SCHEMA_VERSION, CompatibilityStatus, ControlController, ControlLogSnapshot,
    ControlModel, DiagnosticInput, DiscoveryReport, MAX_CONTROL_GAME_NAME_BYTES,
    MAX_CONTROL_LOG_LINE_BYTES, MAX_CONTROL_LOG_LINES, MAX_CONTROL_LOG_RESPONSE_BYTES,
    MAX_CONTROL_SNAPSHOT_BYTES, NativePathValidator, SelectionStore, SteamGame,
};

struct MemoryStore;

impl SelectionStore for MemoryStore {
    fn save(&self, _settings: &LifecycleSettings) -> io::Result<()> {
        Ok(())
    }
}

fn controller_with_game(name: String) -> ControlController {
    let game = SteamGame {
        app_id: 4242,
        name,
        install_dir: PathBuf::from("/games/example"),
        icon: None,
    };
    let mut settings = LifecycleSettings::default();
    settings.selected_steam_app_ids.insert(game.app_id);
    let model = ControlModel::new(
        SettingsLoad {
            settings,
            warning: None,
        },
        DiscoveryReport {
            games: vec![game],
            warnings: Vec::new(),
        },
        NativePathValidator,
    );
    ControlController::new_with_diagnostic_input(
        model,
        MemoryStore,
        DiagnosticInput {
            session_type: Some("wayland".to_owned()),
            current_desktop: Some("Hyprland".to_owned()),
            desktop_session: Some("omarchy".to_owned()),
            ..DiagnosticInput::default()
        },
    )
}

#[test]
fn snapshot_projects_authority_as_bounded_presentation_data() {
    let controller = controller_with_game("Example Game".to_owned());

    let snapshot = controller.snapshot();

    assert_eq!(snapshot.schema_version, 1);
    assert_eq!(
        snapshot.compatibility.status,
        CompatibilityStatus::Supported
    );
    assert!(snapshot.compatibility.activation_allowed);
    assert_eq!(snapshot.games.len(), 1);
    assert_eq!(snapshot.games[0].app_id, 4242);
    assert_eq!(snapshot.games[0].name, "Example Game");
    assert!(snapshot.games[0].selected);
    assert!(!snapshot.operations.any_in_flight());
    assert!(snapshot.selection_editing_enabled);
    assert_eq!(snapshot.shortcut, "Meta+Alt+O");

    let json = serde_json::to_vec(&snapshot).expect("snapshot should serialize");
    assert!(json.len() <= MAX_CONTROL_SNAPSHOT_BYTES);
}

#[test]
fn snapshot_truncates_untrusted_game_labels_on_utf8_boundaries() {
    let controller =
        controller_with_game(format!("{}é", "X".repeat(MAX_CONTROL_GAME_NAME_BYTES * 2)));

    let name = &controller.snapshot().games[0].name;
    assert!(name.len() <= MAX_CONTROL_GAME_NAME_BYTES);
    assert!(name.is_char_boundary(name.len()));
}

#[test]
fn polling_without_workers_is_idle_and_does_not_change_state() {
    let mut controller = controller_with_game("Example Game".to_owned());
    let before = controller.snapshot();

    assert!(!controller.poll_pending());
    assert_eq!(controller.snapshot(), before);
}

#[test]
fn serialized_enums_are_stable_language_neutral_codes() {
    let snapshot = controller_with_game("Example Game".to_owned()).snapshot();
    let json = serde_json::to_value(snapshot).expect("snapshot should serialize");

    assert_eq!(json["compatibility"]["status"], "supported");
    assert_eq!(json["compatibility"]["desktop"], "hyprland");
    assert_eq!(json["compatibility"]["reason"], "hyprland_wayland");
    assert_eq!(json["lifecycle"], "disabled");
}

#[test]
fn recent_logs_keep_the_newest_bounded_lines() {
    let snapshot = ControlLogSnapshot::from_recent_lines(
        (0..=MAX_CONTROL_LOG_LINES)
            .map(|index| {
                format!(
                    "2026-07-23T10:00:{:02}.000Z INFO core event_{index}",
                    index % 60
                )
            })
            .collect(),
    )
    .expect("bounded logs");

    assert_eq!(snapshot.schema_version, CONTROL_LOG_SCHEMA_VERSION);
    assert_eq!(snapshot.lines.len(), MAX_CONTROL_LOG_LINES);
    assert!(!snapshot.lines[0].ends_with("event_0"));
    assert!(snapshot.lines[MAX_CONTROL_LOG_LINES - 1].ends_with("event_500"));
    assert!(snapshot.truncated);
    assert!(snapshot.has_valid_wire_bounds());
    assert!(
        serde_json::to_vec(&snapshot)
            .expect("log snapshot should serialize")
            .len()
            <= MAX_CONTROL_LOG_RESPONSE_BYTES
    );
}

#[test]
fn recent_logs_reject_oversized_lines_and_prune_to_the_wire_limit() {
    assert!(
        ControlLogSnapshot::from_recent_lines(vec!["x".repeat(MAX_CONTROL_LOG_LINE_BYTES + 1)])
            .is_none()
    );
    assert!(
        ControlLogSnapshot::from_recent_lines(vec![
            "2026-07-23T10:00:00.000Z INFO core first\nforged".to_owned()
        ])
        .is_none()
    );

    let prefix = "2026-07-23T10:00:00.000Z INFO core event ";
    let snapshot = ControlLogSnapshot::from_recent_lines(vec![
        format!(
            "{prefix}{}",
            "x".repeat(MAX_CONTROL_LOG_LINE_BYTES - prefix.len())
        );
        MAX_CONTROL_LOG_LINES
    ])
    .expect("bounded logs");

    assert!(snapshot.lines.len() < MAX_CONTROL_LOG_LINES);
    assert!(snapshot.truncated);
    assert!(snapshot.has_valid_wire_bounds());
    assert!(
        serde_json::to_vec(&snapshot)
            .expect("log snapshot should serialize")
            .len()
            <= MAX_CONTROL_LOG_RESPONSE_BYTES
    );
}
