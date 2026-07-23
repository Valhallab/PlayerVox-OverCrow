use std::{
    collections::BTreeSet,
    fs, io,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Barrier, Mutex},
    thread,
    time::Duration,
};

use overcrow_config::{CommittedSettingsSaveError, LifecycleSettings, ManualGame, SettingsLoad};

use crate::{
    integration::IntegrationSetup,
    lifecycle::{
        CommandRunner, CorePassiveClient, DISABLE_CORE, ENABLE_CORE, LifecycleController,
        LifecycleManualIdentityValidator, LifecycleSettingsRepository, LifecycleStatus,
        STOP_SESSION, SYSTEMCTL_PROGRAM, TIMEOUT_PROGRAM, exact_fragment_output,
        is_service_unknown_name, prepare_activation_with, supported_unit_directory,
    },
};

#[derive(Clone, Default)]
struct FakeRepository {
    settings: Arc<Mutex<LifecycleSettings>>,
    events: Arc<Mutex<Vec<String>>>,
    fail_saves: Arc<Mutex<Vec<bool>>>,
}

impl LifecycleSettingsRepository for FakeRepository {
    fn load(&self) -> SettingsLoad {
        SettingsLoad {
            settings: self.settings.lock().unwrap().clone(),
            warning: None,
        }
    }

    fn save(&self, settings: &LifecycleSettings) -> io::Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("save:{}", settings.enabled));
        if self
            .fail_saves
            .lock()
            .unwrap()
            .first()
            .copied()
            .unwrap_or(false)
        {
            self.fail_saves.lock().unwrap().remove(0);
            return Err(io::Error::other("save failed"));
        }
        if !self.fail_saves.lock().unwrap().is_empty() {
            self.fail_saves.lock().unwrap().remove(0);
        }
        *self.settings.lock().unwrap() = settings.clone();
        Ok(())
    }
}

struct FakeIntegration {
    events: Arc<Mutex<Vec<String>>>,
    fail: bool,
}
impl IntegrationSetup for FakeIntegration {
    fn ensure_ready(&self) -> Result<(), String> {
        self.events.lock().unwrap().push("integration".into());
        self.fail
            .then_some(())
            .map_or(Ok(()), |_| Err("integration failed".into()))
    }
}

struct FakeCommands {
    events: Arc<Mutex<Vec<String>>>,
    fail: Option<&'static [&'static str]>,
}
impl CommandRunner for FakeCommands {
    fn prepare_activation(&self) -> Result<(), String> {
        Ok(())
    }

    fn run_systemctl(&self, args: &'static [&'static str]) -> Result<(), String> {
        let label = if args == ENABLE_CORE {
            "enable:core"
        } else if args == DISABLE_CORE {
            "disable:core"
        } else if args == STOP_SESSION {
            "stop:session"
        } else {
            "unknown"
        };
        self.events.lock().unwrap().push(label.into());
        (self.fail == Some(args))
            .then_some(())
            .map_or(Ok(()), |_| Err(format!("{label} failed")))
    }
}

struct FakePassive {
    events: Arc<Mutex<Vec<String>>>,
    result: Result<(), String>,
}
impl CorePassiveClient for FakePassive {
    fn request_passive(&self) -> Result<(), String> {
        self.events.lock().unwrap().push("passive".into());
        self.result.clone()
    }
}

fn selected(enabled: bool) -> LifecycleSettings {
    LifecycleSettings {
        enabled,
        selected_steam_app_ids: BTreeSet::from([620]),
        ..LifecycleSettings::default()
    }
}

fn fixture(
    repository: FakeRepository,
    fail_integration: bool,
    fail_command: Option<&'static [&'static str]>,
    passive: Result<(), String>,
) -> LifecycleController {
    let events = repository.events.clone();
    LifecycleController::injected(
        Arc::new(repository),
        Arc::new(FakeIntegration {
            events: events.clone(),
            fail: fail_integration,
        }),
        Arc::new(FakeCommands {
            events: events.clone(),
            fail: fail_command,
        }),
        Arc::new(FakePassive {
            events,
            result: passive,
        }),
    )
}

#[test]
fn disable_persists_false_before_every_cleanup_and_preserves_selections() {
    let repository = FakeRepository::default();
    *repository.settings.lock().unwrap() = selected(true);
    let controller = fixture(repository.clone(), false, None, Ok(()));

    controller.disable().unwrap();

    assert_eq!(
        *repository.events.lock().unwrap(),
        ["save:false", "passive", "stop:session", "disable:core"]
    );
    let current = repository.settings.lock().unwrap();
    assert!(!current.enabled);
    assert_eq!(current.selected_steam_app_ids, BTreeSet::from([620]));
}

#[test]
fn enable_orders_save_integration_and_core_start() {
    let repository = FakeRepository::default();
    let controller = fixture(repository.clone(), false, None, Ok(()));
    controller.enable(&selected(false)).unwrap();
    assert_eq!(
        *repository.events.lock().unwrap(),
        ["save:true", "integration", "enable:core"]
    );
    assert_eq!(controller.status(), LifecycleStatus::Enabled);
}

struct ActivationPreflightCommands {
    events: Arc<Mutex<Vec<String>>>,
    result: Result<(), String>,
}

impl CommandRunner for ActivationPreflightCommands {
    fn prepare_activation(&self) -> Result<(), String> {
        self.events.lock().unwrap().push("systemd:prepare".into());
        self.result.clone()
    }

    fn run_systemctl(&self, args: &'static [&'static str]) -> Result<(), String> {
        let label = if args == ENABLE_CORE {
            "enable:core"
        } else if args == DISABLE_CORE {
            "disable:core"
        } else if args == STOP_SESSION {
            "stop:session"
        } else {
            "unknown"
        };
        self.events.lock().unwrap().push(label.into());
        Ok(())
    }
}

fn activation_preflight_fixture(
    result: Result<(), String>,
) -> (LifecycleController, FakeRepository) {
    let repository = FakeRepository::default();
    let events = repository.events.clone();
    let controller = LifecycleController::injected(
        Arc::new(repository.clone()),
        Arc::new(FakeIntegration {
            events: events.clone(),
            fail: false,
        }),
        Arc::new(ActivationPreflightCommands {
            events: events.clone(),
            result,
        }),
        Arc::new(FakePassive {
            events,
            result: Ok(()),
        }),
    );
    (controller, repository)
}

#[test]
fn exact_three_unit_fragments_are_verified_before_existing_enable_flow() {
    let (controller, repository) = activation_preflight_fixture(Ok(()));

    controller.enable(&selected(false)).unwrap();

    assert_eq!(
        *repository.events.lock().unwrap(),
        ["systemd:prepare", "save:true", "integration", "enable:core"]
    );
}

#[test]
fn daemon_reload_failure_prevents_every_enable_mutation() {
    let (controller, repository) =
        activation_preflight_fixture(Err("systemd daemon-reload failed".into()));

    let error = controller.enable(&selected(false)).unwrap_err().to_string();

    assert!(error.contains("daemon-reload"));
    assert_eq!(*repository.events.lock().unwrap(), ["systemd:prepare"]);
    assert!(!repository.settings.lock().unwrap().enabled);
}

#[test]
fn each_foreign_unit_fragment_prevents_every_enable_mutation() {
    for unit in [
        "overcrow-core.service",
        "overcrow-overlay.service",
        "overcrow-hyprland.service",
    ] {
        let (controller, repository) =
            activation_preflight_fixture(Err(format!("foreign FragmentPath for {unit}")));

        let error = controller.enable(&selected(false)).unwrap_err().to_string();

        assert!(error.contains(unit));
        assert_eq!(*repository.events.lock().unwrap(), ["systemd:prepare"]);
        assert!(!repository.settings.lock().unwrap().enabled);
    }
}

#[test]
fn supported_install_layouts_resolve_the_only_allowed_unit_directories() {
    assert_eq!(
        supported_unit_directory(
            PathBuf::from("/usr/bin/overcrow-control").as_path(),
            None,
            None,
        )
        .unwrap(),
        PathBuf::from("/usr/lib/systemd/user")
    );
    assert_eq!(
        supported_unit_directory(
            PathBuf::from("/home/test/.local/bin/overcrow-control").as_path(),
            Some(PathBuf::from("/home/test").as_path()),
            Some(PathBuf::from("/srv/test-data").as_path()),
        )
        .unwrap(),
        PathBuf::from("/srv/test-data/systemd/user")
    );
    assert_eq!(
        supported_unit_directory(
            PathBuf::from("/home/test/.local/bin/overcrow-control").as_path(),
            Some(PathBuf::from("/home/test").as_path()),
            None,
        )
        .unwrap(),
        PathBuf::from("/home/test/.local/share/systemd/user")
    );
}

#[test]
fn unsupported_or_ambiguous_install_layouts_have_no_unit_authority() {
    let local = PathBuf::from("/home/test/.local/bin/overcrow-control");

    assert!(
        supported_unit_directory(
            PathBuf::from("/opt/overcrow-control").as_path(),
            Some(PathBuf::from("/home/test").as_path()),
            None,
        )
        .is_err()
    );
    assert!(
        supported_unit_directory(
            local.as_path(),
            Some(PathBuf::from("relative-home").as_path()),
            None,
        )
        .is_err()
    );
    assert!(
        supported_unit_directory(
            local.as_path(),
            Some(PathBuf::from("/home/test").as_path()),
            Some(PathBuf::from("relative-data").as_path()),
        )
        .is_err()
    );
}

#[test]
fn fragment_output_is_accepted_only_as_one_byte_exact_line() {
    let expected = PathBuf::from("/home/test/.local/share/systemd/user/overcrow-core.service");
    let mut exact = expected.as_os_str().as_encoded_bytes().to_vec();
    exact.push(b'\n');

    assert!(exact_fragment_output(&exact, &expected));
    assert!(!exact_fragment_output(
        expected.as_os_str().as_encoded_bytes(),
        &expected
    ));
    assert!(!exact_fragment_output(
        &[exact.as_slice(), b"\n"].concat(),
        &expected
    ));
    assert!(!exact_fragment_output(
        &[exact.as_slice(), b"\0"].concat(),
        &expected
    ));
}

#[test]
fn production_activation_preflight_has_one_fixed_exact_command_sequence() {
    assert_eq!(TIMEOUT_PROGRAM, "/usr/bin/timeout");
    assert_eq!(SYSTEMCTL_PROGRAM, "/usr/bin/systemctl");

    let executable = PathBuf::from("/home/test/.local/bin/overcrow-control");
    let home = PathBuf::from("/home/test");
    let data_home = PathBuf::from("/srv/test-data");
    let mut calls = Vec::new();
    prepare_activation_with(&executable, Some(&home), Some(&data_home), |arguments| {
        calls.push(
            arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect::<Vec<_>>(),
        );
        let Some(unit) = arguments.last() else {
            return Ok(Vec::new());
        };
        if *unit == "daemon-reload" {
            return Ok(Vec::new());
        }
        Ok(format!("/srv/test-data/systemd/user/{unit}\n").into_bytes())
    })
    .unwrap();

    assert_eq!(
        calls,
        [
            vec!["--user", "daemon-reload"],
            vec![
                "--user",
                "show",
                "--property=FragmentPath",
                "--value",
                "overcrow-core.service",
            ],
            vec![
                "--user",
                "show",
                "--property=FragmentPath",
                "--value",
                "overcrow-overlay.service",
            ],
            vec![
                "--user",
                "show",
                "--property=FragmentPath",
                "--value",
                "overcrow-hyprland.service",
            ],
        ]
    );
}

#[test]
fn production_activation_preflight_stops_at_each_foreign_fragment() {
    let executable = PathBuf::from("/usr/bin/overcrow-control");
    let units = [
        "overcrow-core.service",
        "overcrow-overlay.service",
        "overcrow-hyprland.service",
    ];

    for (foreign_index, foreign_unit) in units.iter().enumerate() {
        let mut calls = Vec::new();
        let result = prepare_activation_with(&executable, None, None, |arguments| {
            calls.push(arguments.last().copied().unwrap_or_default().to_owned());
            let unit = arguments.last().copied().unwrap_or_default();
            if unit == "daemon-reload" {
                Ok(Vec::new())
            } else if unit == *foreign_unit {
                Ok(b"/foreign/unit\n".to_vec())
            } else {
                Ok(format!("/usr/lib/systemd/user/{unit}\n").into_bytes())
            }
        });

        assert!(result.unwrap_err().contains(foreign_unit));
        assert_eq!(
            calls,
            [&["daemon-reload"][..], &units[..=foreign_index]].concat()
        );
    }
}

#[test]
fn production_activation_preflight_does_not_query_fragments_after_reload_failure() {
    let executable = PathBuf::from("/usr/bin/overcrow-control");
    let mut calls = Vec::new();

    let error = prepare_activation_with(&executable, None, None, |arguments| {
        calls.push(
            arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect::<Vec<_>>(),
        );
        Err("reload failed".to_owned())
    })
    .unwrap_err();

    assert!(error.contains("daemon-reload"));
    assert_eq!(calls, [vec!["--user", "daemon-reload"]]);
}

#[test]
fn no_selected_identity_cannot_enable() {
    let repository = FakeRepository::default();
    let controller = fixture(repository.clone(), false, None, Ok(()));
    assert!(controller.enable(&LifecycleSettings::default()).is_err());
    assert!(repository.events.lock().unwrap().is_empty());
}

fn manual_settings(path: PathBuf) -> LifecycleSettings {
    LifecycleSettings {
        manual_games: vec![ManualGame {
            id: crate::model::stable_manual_game_id(&path),
            name: "Native game".to_owned(),
            executable: path,
        }],
        ..LifecycleSettings::default()
    }
}

#[test]
fn deleted_manual_identity_rejects_the_entire_enable_before_external_changes() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("native-game");
    fs::write(&executable, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let settings = manual_settings(executable.canonicalize().unwrap());
    fs::remove_file(&executable).unwrap();
    let repository = FakeRepository::default();
    let controller = fixture(repository.clone(), false, None, Ok(()));

    let error = controller.enable(&settings).unwrap_err().to_string();

    assert!(error.contains("manual game identity"));
    assert!(repository.events.lock().unwrap().is_empty());
}

#[test]
fn changed_manual_identity_rejects_enable_before_external_changes() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("native-game");
    fs::write(&executable, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let settings = manual_settings(executable.canonicalize().unwrap());
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o644)).unwrap();
    let repository = FakeRepository::default();
    let controller = fixture(repository.clone(), false, None, Ok(()));

    assert!(controller.enable(&settings).is_err());
    assert!(repository.events.lock().unwrap().is_empty());
}

#[test]
fn canonical_target_replacement_rejects_enable_before_external_changes() {
    let temp = tempfile::tempdir().unwrap();
    let selected_path = temp.path().join("selected-game");
    let replacement_target = temp.path().join("replacement-game");
    fs::write(&selected_path, b"original\n").unwrap();
    fs::set_permissions(&selected_path, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&replacement_target, b"replacement\n").unwrap();
    fs::set_permissions(&replacement_target, fs::Permissions::from_mode(0o755)).unwrap();
    let settings = manual_settings(selected_path.canonicalize().unwrap());
    fs::remove_file(&selected_path).unwrap();
    std::os::unix::fs::symlink(&replacement_target, &selected_path).unwrap();
    let repository = FakeRepository::default();
    let controller = fixture(repository.clone(), false, None, Ok(()));

    let error = controller.enable(&settings).unwrap_err().to_string();

    assert!(error.contains("manual game identity"));
    assert!(repository.events.lock().unwrap().is_empty());
}

#[test]
fn one_invalid_manual_identity_rejects_a_mixed_steam_selection() {
    let missing = PathBuf::from("/missing/overcrow-native-game");
    let mut settings = manual_settings(missing);
    settings.selected_steam_app_ids.insert(620);
    let repository = FakeRepository::default();
    let controller = fixture(repository.clone(), false, None, Ok(()));

    assert!(controller.enable(&settings).is_err());
    assert!(repository.events.lock().unwrap().is_empty());
}

#[test]
fn noncanonical_and_wine_manual_identities_never_reach_save_or_commands() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("native-game");
    let alias = temp.path().join("native-alias");
    fs::write(&target, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let repository = FakeRepository::default();
    let controller = fixture(repository.clone(), false, None, Ok(()));
    assert!(controller.enable(&manual_settings(alias)).is_err());
    assert!(
        controller
            .enable(&manual_settings(PathBuf::from("/games/unsafe.exe")))
            .is_err()
    );
    assert!(repository.events.lock().unwrap().is_empty());
}

#[test]
fn an_injected_manual_validator_runs_inside_enable_before_any_side_effect() {
    struct RejectingValidator;
    impl LifecycleManualIdentityValidator for RejectingValidator {
        fn validate(&self, _game: &ManualGame) -> Result<(), String> {
            Err("injected identity rejection".to_owned())
        }
    }
    let repository = FakeRepository::default();
    let events = repository.events.clone();
    let controller = LifecycleController::injected_with_validator(
        Arc::new(repository),
        Arc::new(FakeIntegration {
            events: events.clone(),
            fail: false,
        }),
        Arc::new(FakeCommands {
            events: events.clone(),
            fail: None,
        }),
        Arc::new(FakePassive {
            events: events.clone(),
            result: Ok(()),
        }),
        Arc::new(RejectingValidator),
    );

    let error = controller
        .enable(&manual_settings(PathBuf::from("/games/native")))
        .unwrap_err()
        .to_string();

    assert!(error.contains("injected identity rejection"));
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn failed_enable_rolls_back_in_fail_closed_order() {
    let repository = FakeRepository::default();
    let controller = fixture(repository.clone(), false, Some(ENABLE_CORE), Ok(()));
    let error = controller.enable(&selected(false)).unwrap_err();
    assert!(error.to_string().contains("enable:core failed"));
    assert_eq!(
        *repository.events.lock().unwrap(),
        [
            "save:true",
            "integration",
            "enable:core",
            "save:false",
            "passive",
            "stop:session",
            "disable:core"
        ]
    );
    assert!(!repository.settings.lock().unwrap().enabled);
    assert_eq!(
        repository.settings.lock().unwrap().selected_steam_app_ids,
        BTreeSet::from([620])
    );
}

#[test]
fn disable_attempts_all_cleanup_and_aggregates_errors() {
    let repository = FakeRepository::default();
    *repository.settings.lock().unwrap() = selected(true);
    let controller = fixture(
        repository.clone(),
        false,
        Some(STOP_SESSION),
        Err("core unavailable".into()),
    );
    let error = controller.disable().unwrap_err().to_string();
    assert!(error.contains("core unavailable"));
    assert!(error.contains("stop:session failed"));
    assert!(
        repository
            .events
            .lock()
            .unwrap()
            .contains(&"disable:core".into())
    );
}

#[test]
fn repeated_disable_still_cleans_stale_external_units() {
    let repository = FakeRepository::default();
    let controller = fixture(repository.clone(), false, None, Ok(()));
    controller.disable().unwrap();
    controller.disable().unwrap();
    assert_eq!(
        repository
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| *event == "stop:session")
            .count(),
        2
    );
}

#[test]
fn lifecycle_transactions_are_serialized() {
    struct BlockingIntegration {
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
    }
    impl IntegrationSetup for BlockingIntegration {
        fn ensure_ready(&self) -> Result<(), String> {
            self.entered.wait();
            self.release.wait();
            Ok(())
        }
    }
    let repository = FakeRepository::default();
    let events = repository.events.clone();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let controller = Arc::new(LifecycleController::injected(
        Arc::new(repository),
        Arc::new(BlockingIntegration {
            entered: entered.clone(),
            release: release.clone(),
        }),
        Arc::new(FakeCommands {
            events: events.clone(),
            fail: None,
        }),
        Arc::new(FakePassive {
            events,
            result: Ok(()),
        }),
    ));
    let first = {
        let controller = controller.clone();
        thread::spawn(move || controller.enable(&selected(false)))
    };
    entered.wait();
    let second = {
        let controller = controller.clone();
        thread::spawn(move || controller.disable())
    };
    thread::sleep(Duration::from_millis(30));
    assert!(!second.is_finished());
    release.wait();
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
}

#[test]
fn service_unknown_is_the_only_unreachable_core_error_treated_as_passive() {
    assert!(is_service_unknown_name(
        "org.freedesktop.DBus.Error.ServiceUnknown"
    ));
    assert!(!is_service_unknown_name(
        "org.freedesktop.DBus.Error.NameHasNoOwner"
    ));
    assert!(!is_service_unknown_name(
        "org.freedesktop.DBus.Error.ServiceUnknown.AttackerSuffix"
    ));
}

struct PostCommitRepository {
    settings: Mutex<LifecycleSettings>,
    events: Arc<Mutex<Vec<String>>>,
    fail_enabled: bool,
    fail_disabled: bool,
}

impl LifecycleSettingsRepository for PostCommitRepository {
    fn load(&self) -> SettingsLoad {
        SettingsLoad {
            settings: self.settings.lock().unwrap().clone(),
            warning: None,
        }
    }

    fn save(&self, settings: &LifecycleSettings) -> io::Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("save:{}", settings.enabled));
        *self.settings.lock().unwrap() = settings.clone();
        if (settings.enabled && self.fail_enabled) || (!settings.enabled && self.fail_disabled) {
            return Err(io::Error::other(CommittedSettingsSaveError::new(
                io::Error::other("parent sync failed"),
            )));
        }
        Ok(())
    }
}

fn post_commit_controller(
    fail_enabled: bool,
    fail_disabled: bool,
) -> (
    LifecycleController,
    Arc<Mutex<Vec<String>>>,
    Arc<PostCommitRepository>,
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let repository = Arc::new(PostCommitRepository {
        settings: Mutex::new(selected(false)),
        events: events.clone(),
        fail_enabled,
        fail_disabled,
    });
    let controller = LifecycleController::injected(
        repository.clone(),
        Arc::new(FakeIntegration {
            events: events.clone(),
            fail: false,
        }),
        Arc::new(FakeCommands {
            events: events.clone(),
            fail: None,
        }),
        Arc::new(FakePassive {
            events: events.clone(),
            result: Ok(()),
        }),
    );
    (controller, events, repository)
}

#[test]
fn committed_but_not_durable_enable_save_rolls_back_before_cleanup() {
    let (controller, events, repository) = post_commit_controller(true, false);

    let error = controller.enable(&selected(false)).unwrap_err().to_string();

    assert!(error.contains("durability"));
    assert_eq!(
        *events.lock().unwrap(),
        [
            "save:true",
            "save:false",
            "passive",
            "stop:session",
            "disable:core"
        ]
    );
    assert!(!repository.settings.lock().unwrap().enabled);
}

#[test]
fn committed_but_not_durable_rollback_false_stays_fail_closed_and_is_reported() {
    let (controller, events, repository) = post_commit_controller(false, true);

    let error = controller.disable().unwrap_err().to_string();

    assert!(error.contains("durability unknown"));
    assert!(!repository.settings.lock().unwrap().enabled);
    assert_eq!(
        *events.lock().unwrap(),
        ["save:false", "passive", "stop:session", "disable:core"]
    );
}

#[test]
fn committed_but_not_durable_false_save_during_enable_rollback_is_aggregated() {
    let (base, events, repository) = post_commit_controller(false, true);
    drop(base);
    let controller = LifecycleController::injected(
        repository.clone(),
        Arc::new(FakeIntegration {
            events: events.clone(),
            fail: true,
        }),
        Arc::new(FakeCommands {
            events: events.clone(),
            fail: None,
        }),
        Arc::new(FakePassive {
            events: events.clone(),
            result: Ok(()),
        }),
    );

    let error = controller.enable(&selected(false)).unwrap_err().to_string();

    assert!(error.contains("integration failed"));
    assert!(error.contains("durability unknown"));
    assert!(!repository.settings.lock().unwrap().enabled);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "save:true",
            "integration",
            "save:false",
            "passive",
            "stop:session",
            "disable:core"
        ]
    );
}

struct WarningRepository {
    events: Arc<Mutex<Vec<String>>>,
}

impl LifecycleSettingsRepository for WarningRepository {
    fn load(&self) -> SettingsLoad {
        SettingsLoad {
            settings: LifecycleSettings::default(),
            warning: Some("invalid settings".to_owned()),
        }
    }

    fn save(&self, _settings: &LifecycleSettings) -> io::Result<()> {
        self.events.lock().unwrap().push("unexpected-save".into());
        Ok(())
    }
}

#[test]
fn status_fails_closed_when_settings_have_a_warning() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let controller = LifecycleController::injected(
        Arc::new(WarningRepository {
            events: events.clone(),
        }),
        Arc::new(FakeIntegration {
            events: events.clone(),
            fail: false,
        }),
        Arc::new(FakeCommands {
            events: events.clone(),
            fail: None,
        }),
        Arc::new(FakePassive {
            events,
            result: Ok(()),
        }),
    );

    assert_eq!(controller.status(), LifecycleStatus::Warning);
}

#[test]
fn warning_disable_preserves_problematic_settings_and_still_runs_all_cleanup() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let controller = LifecycleController::injected(
        Arc::new(WarningRepository {
            events: events.clone(),
        }),
        Arc::new(FakeIntegration {
            events: events.clone(),
            fail: false,
        }),
        Arc::new(FakeCommands {
            events: events.clone(),
            fail: None,
        }),
        Arc::new(FakePassive {
            events: events.clone(),
            result: Ok(()),
        }),
    );

    let error = controller.disable().unwrap_err().to_string();

    assert!(error.contains("preserved"));
    assert_eq!(
        *events.lock().unwrap(),
        ["passive", "stop:session", "disable:core"]
    );
}

struct WarningAfterEnableRepository {
    settings: Mutex<LifecycleSettings>,
    enabled_was_saved: Mutex<bool>,
    events: Arc<Mutex<Vec<String>>>,
}

impl LifecycleSettingsRepository for WarningAfterEnableRepository {
    fn load(&self) -> SettingsLoad {
        if *self.enabled_was_saved.lock().unwrap() {
            SettingsLoad {
                settings: LifecycleSettings::default(),
                warning: Some("settings became unreadable".to_owned()),
            }
        } else {
            SettingsLoad {
                settings: self.settings.lock().unwrap().clone(),
                warning: None,
            }
        }
    }

    fn save(&self, settings: &LifecycleSettings) -> io::Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("save:{}", settings.enabled));
        *self.settings.lock().unwrap() = settings.clone();
        if settings.enabled {
            *self.enabled_was_saved.lock().unwrap() = true;
        }
        Ok(())
    }
}

#[test]
fn rollback_warning_preserves_problematic_data_and_runs_cleanup() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let repository = Arc::new(WarningAfterEnableRepository {
        settings: Mutex::new(selected(false)),
        enabled_was_saved: Mutex::new(false),
        events: events.clone(),
    });
    let controller = LifecycleController::injected(
        repository.clone(),
        Arc::new(FakeIntegration {
            events: events.clone(),
            fail: true,
        }),
        Arc::new(FakeCommands {
            events: events.clone(),
            fail: None,
        }),
        Arc::new(FakePassive {
            events: events.clone(),
            result: Ok(()),
        }),
    );

    let error = controller.enable(&selected(false)).unwrap_err().to_string();

    assert!(error.contains("settings became unreadable"));
    assert!(repository.settings.lock().unwrap().enabled);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "save:true",
            "integration",
            "passive",
            "stop:session",
            "disable:core"
        ]
    );
}
