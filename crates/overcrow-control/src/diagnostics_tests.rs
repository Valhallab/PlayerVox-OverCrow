use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
    path::{Path, PathBuf},
};

use overcrow_config::{LifecycleSettings, SettingsLoad};

use crate::diagnostics::{
    MAX_CONFIG_PATH_BYTES, MAX_DESKTOP_METADATA_BYTES, MAX_DIAGNOSTIC_COUNT,
    MAX_DIAGNOSTIC_DETAIL_BYTES, MAX_DIAGNOSTIC_LABEL_BYTES, MAX_DISCOVERY_WARNINGS,
    MAX_EXECUTABLE_METADATA_CHECKS, MAX_PATH_DISPLAY_BYTES, MAX_PATH_ENTRIES, MAX_RAW_PATH_BYTES,
    MAX_RENDERED_WARNING_AGGREGATE_BYTES, MAX_SESSION_TYPE_BYTES, MAX_SETTINGS_WARNINGS,
    MAX_SOURCE_WARNING_AGGREGATE_BYTES, MAX_WARNING_BYTES, bounded_path_display,
    diagnostic_input_from_environment_with, is_executable_file,
};
use crate::{
    Availability, ControlModel, DiagnosticInput, DiscoveryReport, Level, PathValidator,
    PortalPickerInput, SelectionError, SteamGame, collect_foundation_diagnostics,
};

#[derive(Clone, Copy)]
struct AcceptAbsolutePaths;

impl PathValidator for AcceptAbsolutePaths {
    fn canonical_executable(&self, path: &Path) -> Result<PathBuf, SelectionError> {
        if path.is_absolute() {
            Ok(path.to_owned())
        } else {
            Err(SelectionError::ExecutableNotAbsolute)
        }
    }
}

fn item<'a>(report: &'a crate::DiagnosticReport, label: &str) -> &'a crate::DiagnosticItem {
    report
        .items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("missing diagnostic item {label:?}"))
}

#[test]
fn invalid_settings_are_reported_without_becoming_enabled() {
    let input = DiagnosticInput {
        settings_warning: Some("settings JSON is invalid".into()),
        ..DiagnosticInput::default()
    };

    let report = collect_foundation_diagnostics(input);

    assert_eq!(report.lifecycle_state, "Disabled");
    assert_eq!(item(&report, "Lifecycle settings").level, Level::Warning);
    assert!(
        item(&report, "Lifecycle settings")
            .detail
            .contains("settings JSON is invalid")
    );
}

#[test]
fn wayland_hyprland_session_is_identified_from_injected_metadata() {
    let report = collect_foundation_diagnostics(DiagnosticInput {
        session_type: Some("wayland".into()),
        current_desktop: Some("Hyprland".into()),
        ..DiagnosticInput::default()
    });

    let session = item(&report, "Desktop session");
    assert_eq!(session.level, Level::Ok);
    assert!(session.detail.contains("Wayland"));
    assert!(session.detail.contains("Hyprland"));
}

#[test]
fn wayland_plasma_and_kde_sessions_are_identified() {
    for desktop in ["KDE", "plasma", "GNOME:KDE"] {
        let report = collect_foundation_diagnostics(DiagnosticInput {
            session_type: Some("WAYLAND".into()),
            current_desktop: Some(desktop.into()),
            ..DiagnosticInput::default()
        });

        let session = item(&report, "Desktop session");
        assert_eq!(session.level, Level::Ok, "desktop metadata: {desktop}");
        assert!(session.detail.contains("Plasma/KDE"));
    }
}

#[test]
fn exact_desktop_session_variants_are_identified() {
    for desktop_session in ["hyprland", "plasmawayland", "plasma-wayland", "kde-plasma"] {
        let report = collect_foundation_diagnostics(DiagnosticInput {
            session_type: Some("wayland".into()),
            desktop_session: Some(desktop_session.into()),
            ..DiagnosticInput::default()
        });

        let session = item(&report, "Desktop session");
        assert_eq!(session.level, Level::Ok, "{desktop_session}");
    }
}

#[test]
fn desktop_name_substrings_do_not_impersonate_known_compositors() {
    for desktop in ["notkde", "nohyprland", "almost-plasma"] {
        let report = collect_foundation_diagnostics(DiagnosticInput {
            session_type: Some("wayland".into()),
            current_desktop: Some(desktop.into()),
            ..DiagnosticInput::default()
        });

        let session = item(&report, "Desktop session");
        assert_eq!(session.level, Level::Info, "{desktop}");
        assert!(session.detail.contains("not identified"));
    }
}

#[test]
fn generic_wayland_session_does_not_guess_a_compositor() {
    let report = collect_foundation_diagnostics(DiagnosticInput {
        session_type: Some("wayland".into()),
        current_desktop: Some("sway".into()),
        ..DiagnosticInput::default()
    });

    let session = item(&report, "Desktop session");
    assert_eq!(session.level, Level::Info);
    assert!(session.detail.contains("Wayland"));
    assert!(session.detail.contains("not identified"));
    assert!(!session.detail.contains("Hyprland"));
    assert!(!session.detail.contains("Plasma/KDE"));
}

#[test]
fn x11_session_is_reported_without_using_wayland_desktop_hints() {
    let report = collect_foundation_diagnostics(DiagnosticInput {
        session_type: Some("x11".into()),
        current_desktop: Some("Hyprland".into()),
        ..DiagnosticInput::default()
    });

    let session = item(&report, "Desktop session");
    assert_eq!(session.level, Level::Ok);
    assert!(session.detail.contains("X11"));
    assert!(!session.detail.contains("Hyprland"));
}

#[test]
fn unknown_session_metadata_is_reported_honestly() {
    for session_type in [None, Some("tty".to_owned())] {
        let report = collect_foundation_diagnostics(DiagnosticInput {
            session_type,
            ..DiagnosticInput::default()
        });

        let session = item(&report, "Desktop session");
        assert_eq!(session.level, Level::Info);
        assert!(session.detail.contains("Unknown"));
    }
}

#[test]
fn missing_config_roots_leave_the_settings_path_unavailable() {
    let report = collect_foundation_diagnostics(DiagnosticInput::default());

    let path = item(&report, "Settings path");
    assert_eq!(path.level, Level::Warning);
    assert!(path.detail.contains("unavailable"));
}

#[test]
fn relative_config_roots_are_rejected_without_becoming_authority() {
    let report = collect_foundation_diagnostics(DiagnosticInput {
        home: Some(PathBuf::from("relative-home")),
        xdg_config_home: Some(PathBuf::from("relative-xdg")),
        ..DiagnosticInput::default()
    });

    let path = item(&report, "Settings path");
    assert_eq!(path.level, Level::Warning);
    assert!(path.detail.contains("relative"));
    assert!(!path.detail.contains("relative-xdg/overcrow"));
}

#[test]
fn relative_xdg_config_home_uses_absolute_home_fallback_with_a_warning() {
    let report = collect_foundation_diagnostics(DiagnosticInput {
        home: Some(PathBuf::from("/home/player")),
        xdg_config_home: Some(PathBuf::from("relative-xdg")),
        ..DiagnosticInput::default()
    });

    let path = item(&report, "Settings path");
    assert_eq!(path.level, Level::Warning);
    assert!(
        path.detail
            .contains("/home/player/.config/overcrow/settings.json")
    );
    assert!(path.detail.contains("relative XDG_CONFIG_HOME"));
}

#[test]
fn portal_picker_represents_executable_and_backend_availability() {
    let available = collect_foundation_diagnostics(DiagnosticInput {
        portal_picker: PortalPickerInput {
            portal_executable: Availability::Available,
            backend_executable: Availability::Available,
        },
        ..DiagnosticInput::default()
    });
    let missing = collect_foundation_diagnostics(DiagnosticInput {
        portal_picker: PortalPickerInput {
            portal_executable: Availability::Unavailable,
            backend_executable: Availability::Unavailable,
        },
        ..DiagnosticInput::default()
    });

    assert_eq!(item(&available, "Portal picker").level, Level::Ok);
    assert!(
        item(&available, "Portal picker")
            .detail
            .contains("executables found")
    );
    assert_eq!(item(&missing, "Portal picker").level, Level::Warning);
    assert!(
        item(&missing, "Portal picker")
            .detail
            .contains("not found in bounded PATH metadata")
    );
    assert!(
        item(&missing, "Portal picker")
            .detail
            .contains("not queried")
    );
}

#[test]
fn no_games_and_discovered_games_are_distinguished() {
    let empty = collect_foundation_diagnostics(DiagnosticInput::default());
    let populated = collect_foundation_diagnostics(DiagnosticInput {
        discovered_steam_games: 3,
        ..DiagnosticInput::default()
    });

    assert_eq!(item(&empty, "Steam discovery").level, Level::Info);
    assert!(
        item(&empty, "Steam discovery")
            .detail
            .contains("No Steam games")
    );
    assert_eq!(item(&populated, "Steam discovery").level, Level::Ok);
    assert!(
        item(&populated, "Steam discovery")
            .detail
            .contains("3 Steam games")
    );
}

#[test]
fn partial_steam_discovery_warnings_remain_visible_with_valid_games() {
    let report = collect_foundation_diagnostics(DiagnosticInput {
        discovered_steam_games: 2,
        discovery_warnings: vec!["secondary library could not be parsed".into()],
        ..DiagnosticInput::default()
    });

    assert_eq!(item(&report, "Steam discovery").level, Level::Ok);
    let warning = item(&report, "Steam discovery warning");
    assert_eq!(warning.level, Level::Warning);
    assert!(warning.detail.contains("secondary library"));
}

#[test]
fn steam_and_manual_selection_counts_are_reported_separately() {
    let report = collect_foundation_diagnostics(DiagnosticInput {
        selected_steam_games: 4,
        selected_manual_games: 2,
        ..DiagnosticInput::default()
    });

    let selections = item(&report, "Game selections");
    assert_eq!(selections.level, Level::Info);
    assert!(selections.detail.contains("4 Steam"));
    assert!(selections.detail.contains("2 manual"));
    assert_eq!(report.lifecycle_state, "Disabled");
}

#[test]
fn environment_collection_uses_injected_values_and_executable_metadata() {
    let input = diagnostic_input_from_environment_with(
        |name| match name {
            "XDG_SESSION_TYPE" => Some(OsString::from("wayland")),
            "XDG_CURRENT_DESKTOP" => Some(OsString::from("KDE")),
            "HOME" => Some(OsString::from("/home/player")),
            "XDG_CONFIG_HOME" => Some(OsString::from("/config/player")),
            "PATH" => Some(OsString::from("/usr/bin:/opt/portal/bin")),
            _ => None,
        },
        |candidate| {
            matches!(
                candidate.file_name().and_then(OsStr::to_str),
                Some("xdg-desktop-portal" | "xdg-desktop-portal-kde")
            )
        },
    );

    assert_eq!(input.session_type.as_deref(), Some("wayland"));
    assert_eq!(input.current_desktop.as_deref(), Some("KDE"));
    assert_eq!(input.home, Some(PathBuf::from("/home/player")));
    assert_eq!(input.xdg_config_home, Some(PathBuf::from("/config/player")));
    assert_eq!(
        input.portal_picker,
        PortalPickerInput {
            portal_executable: Availability::Available,
            backend_executable: Availability::Available,
        }
    );
}

#[test]
fn environment_collection_bounds_direct_path_metadata_checks() {
    let path = (0..MAX_PATH_ENTRIES)
        .map(|index| format!("/bounded/path/{index}"))
        .collect::<Vec<_>>()
        .join(":");
    let checks = Cell::new(0);

    let input = diagnostic_input_from_environment_with(
        |name| (name == "PATH").then(|| OsString::from(&path)),
        |_| {
            checks.set(checks.get() + 1);
            false
        },
    );

    assert_eq!(
        input.portal_picker,
        PortalPickerInput {
            portal_executable: Availability::Unavailable,
            backend_executable: Availability::Unavailable,
        }
    );
    assert_eq!(checks.get(), MAX_EXECUTABLE_METADATA_CHECKS);
}

#[test]
fn path_entry_limit_applies_before_relative_entries_are_filtered() {
    let mut entries = vec!["relative"; MAX_PATH_ENTRIES];
    entries.push("/must-not-be-probed");
    let path = entries.join(":");
    let checks = Cell::new(0);

    let input = diagnostic_input_from_environment_with(
        |name| (name == "PATH").then(|| OsString::from(&path)),
        |_| {
            checks.set(checks.get() + 1);
            false
        },
    );

    assert_eq!(checks.get(), 0);
    assert!(input.environment_was_truncated);
}

#[test]
fn oversized_first_path_entry_is_not_split_or_probed_partially() {
    let path = format!(
        "/{}:/must-not-be-probed",
        "x".repeat(MAX_RAW_PATH_BYTES + 32)
    );
    let checks = Cell::new(0);

    let input = diagnostic_input_from_environment_with(
        |name| (name == "PATH").then(|| OsString::from(&path)),
        |_| {
            checks.set(checks.get() + 1);
            false
        },
    );

    assert_eq!(checks.get(), 0);
    assert!(input.environment_was_truncated);
    let report = collect_foundation_diagnostics(input);
    assert_eq!(
        report
            .items
            .iter()
            .filter(|item| item.label == "Diagnostic bounds")
            .count(),
        1
    );
}

#[test]
fn relative_path_entries_never_reach_executable_metadata() {
    let candidates = RefCell::new(Vec::new());

    let input = diagnostic_input_from_environment_with(
        |name| (name == "PATH").then(|| OsString::from("relative:/absolute")),
        |candidate| {
            candidates.borrow_mut().push(candidate.to_owned());
            false
        },
    );

    assert_eq!(
        input.portal_picker,
        PortalPickerInput {
            portal_executable: Availability::Unavailable,
            backend_executable: Availability::Unavailable,
        }
    );
    assert!(!candidates.borrow().is_empty());
    assert!(
        candidates
            .borrow()
            .iter()
            .all(|candidate| candidate.starts_with("/absolute"))
    );
}

#[test]
fn executable_metadata_follows_symlinks_to_executable_regular_files_only() {
    let temp = tempfile::tempdir().unwrap();
    let real_bin = temp.path().join("real-bin");
    fs::create_dir(&real_bin).unwrap();
    let executable = real_bin.join("portal");
    fs::write(&executable, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let non_executable = real_bin.join("plain-file");
    fs::write(&non_executable, b"plain").unwrap();
    fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o644)).unwrap();
    let executable_link = real_bin.join("portal-link");
    symlink(&executable, &executable_link).unwrap();
    let bin_link = temp.path().join("bin-link");
    symlink(&real_bin, &bin_link).unwrap();

    assert!(is_executable_file(&executable));
    assert!(is_executable_file(&executable_link));
    assert!(is_executable_file(&bin_link.join("portal")));
    assert!(!is_executable_file(&real_bin));
    assert!(!is_executable_file(&non_executable));
}

#[test]
fn environment_strings_and_paths_are_bounded_before_storage() {
    let private_suffix = "PRIVATE_DISCARDED_SUFFIX";
    let session = format!(
        "{}{}",
        "w".repeat(MAX_SESSION_TYPE_BYTES + 8),
        private_suffix
    );
    let desktop = format!(
        "{}{}",
        "é".repeat(MAX_DESKTOP_METADATA_BYTES),
        private_suffix
    );
    let config_path = format!(
        "/{}{}",
        "c".repeat(MAX_CONFIG_PATH_BYTES + 8),
        private_suffix
    );

    let input = diagnostic_input_from_environment_with(
        |name| match name {
            "XDG_SESSION_TYPE" => Some(OsString::from(&session)),
            "XDG_CURRENT_DESKTOP" => Some(OsString::from(&desktop)),
            "DESKTOP_SESSION" => Some(OsString::from(&desktop)),
            "HOME" => Some(OsString::from(&config_path)),
            _ => None,
        },
        |_| panic!("PATH is absent, so metadata must not be inspected"),
    );

    assert!(input.environment_was_truncated);
    assert!(input.session_type.as_ref().unwrap().len() <= MAX_SESSION_TYPE_BYTES);
    assert!(input.current_desktop.as_ref().unwrap().len() <= MAX_DESKTOP_METADATA_BYTES);
    assert!(input.desktop_session.as_ref().unwrap().len() <= MAX_DESKTOP_METADATA_BYTES);
    assert!(input.home.is_none());

    let report = collect_foundation_diagnostics(input);
    assert_eq!(
        report
            .items
            .iter()
            .filter(|item| item.label == "Diagnostic bounds")
            .count(),
        1
    );
    assert!(
        report
            .items
            .iter()
            .all(|item| !item.detail.contains(private_suffix))
    );
}

#[test]
fn warning_count_size_and_aggregate_limits_preserve_utf8_boundaries() {
    let unicode_warning = "é".repeat(MAX_WARNING_BYTES);
    let discovery_warnings = (0..MAX_DISCOVERY_WARNINGS + 8)
        .map(|index| format!("warning-{index}-{unicode_warning}"))
        .collect();

    let report = collect_foundation_diagnostics(DiagnosticInput {
        settings_warning: Some(unicode_warning),
        discovery_warnings,
        ..DiagnosticInput::default()
    });

    assert!(
        report
            .items
            .iter()
            .filter(|item| item.label == "Lifecycle settings")
            .count()
            <= MAX_SETTINGS_WARNINGS
    );
    let discovery = report
        .items
        .iter()
        .filter(|item| item.label == "Steam discovery warning")
        .collect::<Vec<_>>();
    assert!(discovery.len() <= MAX_DISCOVERY_WARNINGS);
    assert!(
        report
            .items
            .iter()
            .filter(|item| {
                item.label == "Lifecycle settings" || item.label == "Steam discovery warning"
            })
            .all(|item| item.detail.len() <= MAX_WARNING_BYTES
                && !item.detail.contains('\u{fffd}'))
    );
    assert!(
        report
            .items
            .iter()
            .filter(|item| item.level == Level::Warning)
            .map(|item| item.detail.len())
            .sum::<usize>()
            <= MAX_RENDERED_WARNING_AGGREGATE_BYTES
    );
    assert_eq!(
        report
            .items
            .iter()
            .filter(|item| item.label == "Diagnostic bounds")
            .count(),
        1
    );
    let settings = item(&report, "Lifecycle settings");
    assert!(settings.detail.is_char_boundary(settings.detail.len()));
    assert_eq!(settings.detail, "é".repeat(MAX_WARNING_BYTES / 2));
}

#[test]
fn model_warning_snapshot_is_bounded_before_clone() {
    let large_warning = "w".repeat(MAX_WARNING_BYTES * 4);
    let model = ControlModel::new(
        SettingsLoad {
            settings: LifecycleSettings::default(),
            warning: Some(large_warning.clone()),
        },
        DiscoveryReport {
            games: Vec::new(),
            warnings: vec![large_warning; MAX_DISCOVERY_WARNINGS + 8],
        },
        AcceptAbsolutePaths,
    );
    let mut input = DiagnosticInput::default();

    input.sync_model(&model);

    assert!(input.model_was_truncated);
    assert!(input.settings_warning.as_ref().unwrap().len() <= MAX_WARNING_BYTES);
    assert!(input.discovery_warnings.len() <= MAX_DISCOVERY_WARNINGS);
    assert!(
        input
            .discovery_warnings
            .iter()
            .all(|warning| warning.len() <= MAX_WARNING_BYTES)
    );
    assert!(
        input
            .settings_warning
            .iter()
            .map(String::len)
            .sum::<usize>()
            + input
                .discovery_warnings
                .iter()
                .map(String::len)
                .sum::<usize>()
            <= MAX_SOURCE_WARNING_AGGREGATE_BYTES
    );
}

#[test]
fn every_rendered_diagnostic_string_has_an_explicit_byte_bound() {
    let report = collect_foundation_diagnostics(DiagnosticInput {
        session_type: Some("wayland".repeat(MAX_DIAGNOSTIC_DETAIL_BYTES)),
        current_desktop: Some("Hyprland".repeat(MAX_DIAGNOSTIC_DETAIL_BYTES)),
        home: Some(PathBuf::from(format!(
            "/{}",
            "home".repeat(MAX_DIAGNOSTIC_DETAIL_BYTES)
        ))),
        settings_warning: Some("settings".repeat(MAX_DIAGNOSTIC_DETAIL_BYTES)),
        discovery_warnings: vec![
            "discovery".repeat(MAX_DIAGNOSTIC_DETAIL_BYTES);
            MAX_DISCOVERY_WARNINGS + 8
        ],
        ..DiagnosticInput::default()
    });

    assert!(
        report
            .items
            .iter()
            .all(|item| item.label.len() <= MAX_DIAGNOSTIC_LABEL_BYTES
                && item.detail.len() <= MAX_DIAGNOSTIC_DETAIL_BYTES)
    );
    assert_eq!(
        report
            .items
            .iter()
            .filter(|item| item.label == "Diagnostic bounds")
            .count(),
        1
    );
}

#[test]
fn normalization_consumes_and_bounds_every_owned_input_field() {
    let huge = "é".repeat(MAX_DIAGNOSTIC_DETAIL_BYTES * 4);
    let input = DiagnosticInput {
        session_type: Some(huge.clone()),
        current_desktop: Some(huge.clone()),
        desktop_session: Some(huge.clone()),
        home: Some(PathBuf::from(format!(
            "/{}",
            "h".repeat(MAX_CONFIG_PATH_BYTES + 1)
        ))),
        xdg_config_home: Some(PathBuf::from(format!(
            "/{}",
            "x".repeat(MAX_CONFIG_PATH_BYTES + 1)
        ))),
        settings_warning: Some(huge.clone()),
        discovery_warnings: vec![huge; MAX_DISCOVERY_WARNINGS + 8],
        discovered_steam_games: usize::MAX,
        selected_steam_games: usize::MAX,
        selected_manual_games: usize::MAX,
        ..DiagnosticInput::default()
    };

    let normalized = input.normalize();

    assert!(normalized.session_type.unwrap().len() <= MAX_SESSION_TYPE_BYTES);
    assert!(normalized.current_desktop.unwrap().len() <= MAX_DESKTOP_METADATA_BYTES);
    assert!(normalized.desktop_session.unwrap().len() <= MAX_DESKTOP_METADATA_BYTES);
    assert!(normalized.home.is_none());
    assert!(normalized.xdg_config_home.is_none());
    assert!(normalized.settings_warning.unwrap().len() <= MAX_WARNING_BYTES);
    assert!(normalized.discovery_warnings.len() <= MAX_DISCOVERY_WARNINGS);
    assert!(
        normalized
            .discovery_warnings
            .iter()
            .all(|warning| warning.len() <= MAX_WARNING_BYTES)
    );
    assert_eq!(normalized.discovered_steam_games, MAX_DIAGNOSTIC_COUNT);
    assert_eq!(normalized.selected_steam_games, MAX_DIAGNOSTIC_COUNT);
    assert_eq!(normalized.selected_manual_games, MAX_DIAGNOSTIC_COUNT);
    assert!(normalized.environment_was_truncated);
    assert!(normalized.model_was_truncated);
}

#[test]
fn non_utf8_config_roots_never_become_lossy_ok_authority() {
    let non_utf8 = OsString::from_vec(b"/home/player/\xff".to_vec());
    let input = diagnostic_input_from_environment_with(
        |name| match name {
            "HOME" | "XDG_CONFIG_HOME" => Some(non_utf8.clone()),
            _ => None,
        },
        |_| panic!("PATH is absent, so metadata must not be inspected"),
    );

    assert!(input.home.is_none());
    assert!(input.xdg_config_home.is_none());
    assert!(input.environment_was_truncated);
    let report = collect_foundation_diagnostics(input);
    let settings = item(&report, "Settings path");
    assert_eq!(settings.level, Level::Warning);
    assert!(!settings.detail.contains("Using"));
    assert!(!settings.detail.contains('\u{fffd}'));
    assert_eq!(
        report
            .items
            .iter()
            .filter(|item| item.label == "Diagnostic bounds")
            .count(),
        1
    );
}

#[test]
fn bounded_path_display_marks_lossy_conversion_even_within_byte_limit() {
    let path = PathBuf::from(OsString::from_vec(b"/short/\xff".to_vec()));

    let (_display, was_lossy) = bounded_path_display(&path, MAX_PATH_DISPLAY_BYTES);

    assert!(was_lossy);
}

#[test]
fn diagnostic_snapshot_syncs_only_bounded_model_results_and_counts() {
    let mut selected_steam_app_ids = BTreeSet::new();
    selected_steam_app_ids.insert(620);
    let settings = LifecycleSettings {
        selected_steam_app_ids,
        ..LifecycleSettings::default()
    };
    let mut model = ControlModel::new(
        SettingsLoad {
            settings,
            warning: Some("settings warning".into()),
        },
        DiscoveryReport {
            games: vec![
                SteamGame {
                    app_id: 620,
                    name: "Portal 2".into(),
                    install_dir: PathBuf::from("/steam/portal2"),
                    icon: None,
                },
                SteamGame {
                    app_id: 1_623_730,
                    name: "Palworld".into(),
                    install_dir: PathBuf::from("/steam/palworld"),
                    icon: None,
                },
            ],
            warnings: vec!["partial discovery".into()],
        },
        AcceptAbsolutePaths,
    );
    model
        .add_manual_game("Portal", Path::new("/games/portal"))
        .unwrap();
    let mut input = DiagnosticInput::default();

    input.sync_model(&model);

    assert_eq!(input.settings_warning.as_deref(), Some("settings warning"));
    assert_eq!(input.discovery_warnings, ["partial discovery"]);
    assert_eq!(input.discovered_steam_games, 2);
    assert_eq!(input.selected_steam_games, 1);
    assert_eq!(input.selected_manual_games, 1);
}

#[test]
fn model_sync_preserves_a_prior_normalization_truncation() {
    let model = ControlModel::new(
        SettingsLoad {
            settings: LifecycleSettings::default(),
            warning: None,
        },
        DiscoveryReport::default(),
        AcceptAbsolutePaths,
    );
    let mut input = DiagnosticInput {
        model_was_truncated: true,
        ..DiagnosticInput::default()
    };

    input.sync_model(&model);

    assert!(input.model_was_truncated);
    let report = collect_foundation_diagnostics(input);
    assert_eq!(
        report
            .items
            .iter()
            .filter(|item| item.label == "Diagnostic bounds")
            .count(),
        1
    );
}
