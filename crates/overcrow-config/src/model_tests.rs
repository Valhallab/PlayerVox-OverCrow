use std::path::PathBuf;

use serde_json::{Value, json};

use crate::{
    LifecycleSettings, ManualGame, SETTINGS_SCHEMA_VERSION, SettingsError, ShortcutSettings,
};

fn settings_json() -> Value {
    json!({
        "schema_version": SETTINGS_SCHEMA_VERSION,
        "enabled": false,
        "selected_steam_app_ids": [],
        "manual_games": [],
        "shortcut": {
            "enabled": true,
            "accelerator": "Meta+Alt+O"
        }
    })
}

#[test]
fn defaults_are_disabled_and_keep_the_default_shortcut() {
    let settings = LifecycleSettings::default();

    assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
    assert!(!settings.enabled);
    assert!(settings.selected_steam_app_ids.is_empty());
    assert!(settings.manual_games.is_empty());
    assert!(settings.shortcut.enabled);
    assert_eq!(settings.shortcut.accelerator, "Meta+Alt+O");
}

#[test]
fn deserialization_rejects_unknown_fields_at_every_model_level() {
    let mut top_level = settings_json();
    top_level["unexpected"] = json!(true);

    let mut manual_game = settings_json();
    manual_game["manual_games"] = json!([{
        "id": "local.portal",
        "name": "Portal",
        "executable": "/games/portal",
        "unexpected": true
    }]);

    let mut shortcut = settings_json();
    shortcut["shortcut"]["unexpected"] = json!(true);

    for invalid in [top_level, manual_game, shortcut] {
        assert!(serde_json::from_value::<LifecycleSettings>(invalid).is_err());
    }
}

#[test]
fn validation_rejects_every_unsupported_schema_version() {
    for version in [0, SETTINGS_SCHEMA_VERSION + 1, u32::MAX] {
        let settings = LifecycleSettings {
            schema_version: version,
            ..LifecycleSettings::default()
        };

        assert_eq!(
            settings.validate(),
            Err(SettingsError::UnsupportedSchemaVersion)
        );
    }
}

#[test]
fn validation_rejects_zero_steam_app_ids() {
    let mut settings = LifecycleSettings::default();
    settings.selected_steam_app_ids.insert(0);

    assert_eq!(settings.validate(), Err(SettingsError::ZeroSteamAppId));
}

#[test]
fn deserialization_rejects_duplicate_steam_app_ids() {
    let mut value = settings_json();
    value["selected_steam_app_ids"] = json!([620, 1623730, 620]);

    let error = serde_json::from_value::<LifecycleSettings>(value).unwrap_err();

    assert!(error.to_string().contains("duplicate Steam app ID 620"));
}

#[test]
fn ordered_steam_ids_have_deterministic_json() {
    let mut settings = LifecycleSettings::default();
    settings.selected_steam_app_ids.extend([1623730, 620]);

    let value = serde_json::to_value(settings).unwrap();

    assert_eq!(value["selected_steam_app_ids"], json!([620, 1623730]));
}

#[test]
fn validation_rejects_duplicate_manual_ids() {
    let settings = LifecycleSettings {
        manual_games: vec![
            ManualGame {
                id: "local.portal".into(),
                name: "Portal".into(),
                executable: PathBuf::from("/games/portal"),
            },
            ManualGame {
                id: "local.portal".into(),
                name: "Portal 2".into(),
                executable: PathBuf::from("/games/portal2"),
            },
        ],
        ..LifecycleSettings::default()
    };

    assert_eq!(settings.validate(), Err(SettingsError::DuplicateManualId));
}

#[test]
fn validation_rejects_relative_manual_executables() {
    let mut settings = LifecycleSettings::default();
    settings.manual_games.push(ManualGame {
        id: "local.portal".into(),
        name: "Portal".into(),
        executable: PathBuf::from("portal"),
    });

    assert_eq!(
        settings.validate(),
        Err(SettingsError::NonAbsoluteExecutable)
    );
}

#[test]
fn validation_accepts_each_supported_accelerator_modifier() {
    for accelerator in [
        "Meta+O",
        "Ctrl+1",
        "Alt+Z",
        "Shift+9",
        "Meta+Ctrl+Alt+Shift+O",
    ] {
        let settings = LifecycleSettings {
            shortcut: ShortcutSettings {
                enabled: true,
                accelerator: accelerator.into(),
            },
            ..LifecycleSettings::default()
        };

        assert_eq!(settings.clone().validate(), Ok(settings));
    }
}

#[test]
fn validation_rejects_malformed_accelerators() {
    for accelerator in [
        "",
        "O",
        "Meta",
        "Meta+",
        "+O",
        "Meta++O",
        "Super+O",
        "meta+O",
        "O+Meta",
        "Meta+O+P",
        "Meta+F1",
        "Meta+?",
        "Meta+Alt+Alt+O",
        "Alt+Meta+O",
        "Shift+Ctrl+O",
        "Meta+é",
        " Meta+O",
        "Meta+O ",
    ] {
        let settings = LifecycleSettings {
            shortcut: ShortcutSettings {
                enabled: true,
                accelerator: accelerator.into(),
            },
            ..LifecycleSettings::default()
        };

        assert_eq!(
            settings.validate(),
            Err(SettingsError::MalformedAccelerator),
            "accelerator {accelerator:?} must be rejected"
        );
    }
}
