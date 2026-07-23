use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Barrier, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use overcrow_config::{CommittedSettingsSaveError, LifecycleSettings, SettingsLoad, SettingsStore};
use tempfile::TempDir;

use crate::app::picker_result;
use crate::diagnostics::{
    MAX_DESKTOP_METADATA_BYTES, MAX_DIAGNOSTIC_DETAIL_BYTES, MAX_DIAGNOSTIC_LABEL_BYTES,
    MAX_DISCOVERY_WARNINGS, MAX_SESSION_TYPE_BYTES, MAX_SOURCE_WARNING_AGGREGATE_BYTES,
    MAX_WARNING_BYTES,
};
use crate::{
    APPLICATION_ID, APPLICATION_TITLE, AppTab, CommandRunner, ControlController, ControlModel,
    ControlView, CorePassiveClient, DiagnosticInput, DiscoveryReport, IntegrationSetup, Level,
    LifecycleController, LifecycleSettingsRepository, NativePathValidator, NoticeOperation,
    PickerFailure, PickerResult, SaveOutcome, SelectionStore, ShortcutDisplay, SteamGame,
    UiNoticeLevel, WorkStart,
};

#[derive(Clone)]
struct UiLifecycleRepository {
    settings: Arc<Mutex<LifecycleSettings>>,
}

struct UiWarningRepository {
    saves: Arc<Mutex<usize>>,
}

impl LifecycleSettingsRepository for UiWarningRepository {
    fn load(&self) -> SettingsLoad {
        SettingsLoad {
            settings: LifecycleSettings::default(),
            warning: Some("unsafe settings file".to_owned()),
        }
    }

    fn save(&self, _settings: &LifecycleSettings) -> io::Result<()> {
        *self.saves.lock().unwrap() += 1;
        Ok(())
    }
}

impl LifecycleSettingsRepository for UiLifecycleRepository {
    fn load(&self) -> SettingsLoad {
        SettingsLoad {
            settings: self.settings.lock().unwrap().clone(),
            warning: None,
        }
    }

    fn save(&self, settings: &LifecycleSettings) -> io::Result<()> {
        *self.settings.lock().unwrap() = settings.clone();
        Ok(())
    }
}

struct UiIntegration {
    entered: Option<Arc<Barrier>>,
    release: Option<Arc<Barrier>>,
    fail: bool,
}

impl IntegrationSetup for UiIntegration {
    fn ensure_ready(&self) -> Result<(), String> {
        if let Some(entered) = &self.entered {
            entered.wait();
        }
        if let Some(release) = &self.release {
            release.wait();
        }
        if self.fail {
            Err("integration failed".to_owned())
        } else {
            Ok(())
        }
    }
}

struct UiCommands;
impl CommandRunner for UiCommands {
    fn prepare_activation(&self) -> Result<(), String> {
        Ok(())
    }

    fn run_systemctl(&self, _args: &'static [&'static str]) -> Result<(), String> {
        Ok(())
    }
}

struct UiPassive;
impl CorePassiveClient for UiPassive {
    fn request_passive(&self) -> Result<(), String> {
        Ok(())
    }
}

fn ui_lifecycle(
    settings: LifecycleSettings,
    integration: UiIntegration,
) -> (Arc<LifecycleController>, Arc<Mutex<LifecycleSettings>>) {
    let shared = Arc::new(Mutex::new(settings));
    let lifecycle = LifecycleController::injected(
        Arc::new(UiLifecycleRepository {
            settings: shared.clone(),
        }),
        Arc::new(integration),
        Arc::new(UiCommands),
        Arc::new(UiPassive),
    );
    (Arc::new(lifecycle), shared)
}

fn lifecycle_controller(
    lifecycle: Arc<LifecycleController>,
    model: ControlModel,
) -> ControlController {
    ControlController::new_with_lifecycle(
        model,
        ControlledStore(StoreBehavior::Save),
        DiagnosticInput::default(),
        lifecycle,
    )
}

#[derive(Clone, Copy)]
enum StoreBehavior {
    Save,
    FailBeforeCommit,
    FailAfterCommit,
}

struct ControlledStore(StoreBehavior);

impl SelectionStore for ControlledStore {
    fn save(&self, _settings: &LifecycleSettings) -> io::Result<()> {
        match self.0 {
            StoreBehavior::Save => Ok(()),
            StoreBehavior::FailBeforeCommit => Err(io::Error::other("forced pre-commit failure")),
            StoreBehavior::FailAfterCommit => Err(io::Error::other(
                CommittedSettingsSaveError::new(io::Error::other("forced parent sync failure")),
            )),
        }
    }
}

fn game(app_id: u32, name: &str) -> SteamGame {
    SteamGame {
        app_id,
        name: name.to_owned(),
        install_dir: PathBuf::from(format!("/games/{name}")),
        icon: None,
    }
}

fn model_with(settings: LifecycleSettings, games: Vec<SteamGame>) -> ControlModel {
    ControlModel::new(
        SettingsLoad {
            settings,
            warning: None,
        },
        DiscoveryReport {
            games,
            warnings: Vec::new(),
        },
        NativePathValidator,
    )
}

fn selected_model() -> ControlModel {
    let mut settings = LifecycleSettings::default();
    settings.selected_steam_app_ids.insert(620);
    model_with(settings, vec![game(620, "Portal 2")])
}

fn controller(temp: &TempDir, model: ControlModel) -> ControlController {
    ControlController::new(
        model,
        SettingsStore::from_path(temp.path().join("overcrow/settings.json")),
    )
}

fn controlled_controller(model: ControlModel, behavior: StoreBehavior) -> ControlController {
    ControlController::new(model, ControlledStore(behavior))
}

fn poll_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "worker result timed out");
        thread::yield_now();
    }
}

#[test]
fn shell_metadata_has_the_exact_application_identity_and_title() {
    assert_eq!(APPLICATION_ID, "com.playervox.OverCrow");
    assert_eq!(APPLICATION_TITLE, "PlayerVox OverCrow");
}

#[test]
fn projection_exposes_all_control_center_tabs() {
    let view = ControlView::from_model(&selected_model());

    assert_eq!(
        view.tabs,
        [
            AppTab::Games,
            AppTab::Settings,
            AppTab::Diagnostics,
            AppTab::About,
        ]
    );
}

#[test]
fn activation_is_unavailable_in_the_foundation_increment() {
    let view = ControlView::from_model(&selected_model());

    assert!(!view.master_switch_enabled);
    assert!(!view.master_switch_checked);
    assert_eq!(
        view.lifecycle_label,
        "Configured — activation not installed"
    );
}

#[test]
fn master_switch_requests_cannot_mutate_lifecycle_authority() {
    let temp = tempfile::tempdir().unwrap();
    let mut controller = controller(&temp, selected_model());

    assert!(!controller.request_master_toggle(true));
    assert!(!controller.model().settings.enabled);
    assert!(!controller.view().master_switch_checked);
}

#[test]
fn lifecycle_enable_runs_off_thread_and_locks_selection_editing() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let initial = selected_model();
    let (lifecycle, _) = ui_lifecycle(
        initial.settings.clone(),
        UiIntegration {
            entered: Some(entered.clone()),
            release: Some(release.clone()),
            fail: false,
        },
    );
    let mut controller = lifecycle_controller(lifecycle, initial);

    assert!(controller.request_master_toggle(true));
    entered.wait();
    assert!(controller.lifecycle_in_flight());
    assert!(!controller.selection_editing_enabled());
    assert_eq!(controller.view().lifecycle_label, "Enabling…");
    assert!(!controller.view().master_switch_enabled);
    assert!(!controller.request_master_toggle(false));
    assert_eq!(
        controller.set_steam_selected(620, false),
        SaveOutcome::Unchanged
    );

    release.wait();
    poll_until(Duration::from_secs(2), || controller.poll_lifecycle());
    assert_eq!(controller.view().lifecycle_label, "Enabled");
    assert!(controller.view().master_switch_checked);
    assert!(!controller.selection_editing_enabled());
    assert_eq!(controller.diagnostic_report().lifecycle_state, "Enabled");
}

#[test]
fn lifecycle_failure_reloads_disabled_settings_and_uses_its_own_notice() {
    let initial = selected_model();
    let (lifecycle, persisted) = ui_lifecycle(
        initial.settings.clone(),
        UiIntegration {
            entered: None,
            release: None,
            fail: true,
        },
    );
    let mut controller = lifecycle_controller(lifecycle, initial);

    assert!(controller.request_master_toggle(true));
    poll_until(Duration::from_secs(2), || controller.poll_lifecycle());

    assert!(!persisted.lock().unwrap().enabled);
    assert!(!controller.model().settings.enabled);
    assert_eq!(controller.view().lifecycle_label, "Disabled");
    let notice = controller.notice().unwrap();
    assert_eq!(notice.operation, NoticeOperation::Lifecycle);
    assert!(notice.message.contains("integration failed"));
}

#[test]
fn lifecycle_disable_shows_disabling_and_preserves_selected_games() {
    let mut settings = selected_model().settings.clone();
    settings.enabled = true;
    let model = model_with(settings.clone(), vec![game(620, "Portal 2")]);
    let (lifecycle, persisted) = ui_lifecycle(
        settings,
        UiIntegration {
            entered: None,
            release: None,
            fail: false,
        },
    );
    let mut controller = lifecycle_controller(lifecycle, model);
    assert_eq!(controller.view().lifecycle_label, "Enabled");

    assert!(controller.request_master_toggle(false));
    assert_eq!(controller.view().lifecycle_label, "Disabling…");
    poll_until(Duration::from_secs(2), || controller.poll_lifecycle());

    assert_eq!(controller.view().lifecycle_label, "Disabled");
    assert_eq!(
        persisted.lock().unwrap().selected_steam_app_ids,
        std::collections::BTreeSet::from([620])
    );
    assert!(controller.selection_editing_enabled());
}

#[test]
fn dropping_controller_waits_for_an_owned_lifecycle_worker() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let initial = selected_model();
    let (lifecycle, _) = ui_lifecycle(
        initial.settings.clone(),
        UiIntegration {
            entered: Some(entered.clone()),
            release: Some(release.clone()),
            fail: false,
        },
    );
    let mut controller = lifecycle_controller(lifecycle, initial);
    assert!(controller.request_master_toggle(true));
    entered.wait();

    let releaser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        release.wait();
    });
    let started = Instant::now();
    drop(controller);
    assert!(started.elapsed() >= Duration::from_millis(25));
    releaser.join().unwrap();
}

#[test]
fn master_switch_is_disabled_without_an_exact_game_selection() {
    let model = model_with(LifecycleSettings::default(), Vec::new());
    let (lifecycle, _) = ui_lifecycle(
        LifecycleSettings::default(),
        UiIntegration {
            entered: None,
            release: None,
            fail: false,
        },
    );
    let mut controller = lifecycle_controller(lifecycle, model);

    assert!(!controller.view().master_switch_enabled);
    assert!(!controller.request_master_toggle(true));
    assert_eq!(
        controller.notice().unwrap().operation,
        NoticeOperation::Lifecycle
    );
}

#[test]
fn raw_enabled_status_locks_edits_after_model_drops_an_invalid_manual_identity() {
    let raw = LifecycleSettings {
        enabled: true,
        manual_games: vec![overcrow_config::ManualGame {
            id: "local.missing".to_owned(),
            name: "Missing game".to_owned(),
            executable: PathBuf::from("/missing/overcrow-native-game"),
        }],
        ..LifecycleSettings::default()
    };
    let model = model_with(raw.clone(), Vec::new());
    assert!(!model.settings.enabled);
    assert!(model.settings.manual_games.is_empty());
    let (lifecycle, persisted) = ui_lifecycle(
        raw,
        UiIntegration {
            entered: None,
            release: None,
            fail: false,
        },
    );
    let mut controller = lifecycle_controller(lifecycle, model);

    assert_eq!(controller.view().lifecycle_label, "Enabled");
    assert!(controller.view().master_switch_checked);
    assert!(!controller.selection_editing_enabled());
    assert_eq!(
        controller.set_steam_selected(620, true),
        SaveOutcome::Unchanged
    );

    assert!(controller.request_master_toggle(false));
    poll_until(Duration::from_secs(2), || controller.poll_lifecycle());
    assert!(!persisted.lock().unwrap().enabled);
    assert!(controller.selection_editing_enabled());
}

#[test]
fn warning_status_is_checked_cleanup_capable_and_never_enable_capable() {
    let saves = Arc::new(Mutex::new(0));
    let lifecycle = Arc::new(LifecycleController::injected(
        Arc::new(UiWarningRepository {
            saves: saves.clone(),
        }),
        Arc::new(UiIntegration {
            entered: None,
            release: None,
            fail: false,
        }),
        Arc::new(UiCommands),
        Arc::new(UiPassive),
    ));
    let mut controller = lifecycle_controller(
        lifecycle,
        model_with(LifecycleSettings::default(), Vec::new()),
    );

    assert_eq!(
        controller.view().lifecycle_label,
        "Cleanup required (settings warning)"
    );
    assert!(controller.view().master_switch_checked);
    assert!(controller.view().master_switch_enabled);
    assert!(!controller.selection_editing_enabled());
    assert!(!controller.request_master_toggle(true));
    assert!(!controller.lifecycle_in_flight());

    assert!(controller.request_master_toggle(false));
    poll_until(Duration::from_secs(2), || controller.poll_lifecycle());
    assert_eq!(*saves.lock().unwrap(), 0);
    assert_eq!(
        controller.view().lifecycle_label,
        "Cleanup required (settings warning)"
    );
    assert!(controller.request_master_toggle(false));
    poll_until(Duration::from_secs(2), || controller.poll_lifecycle());
}

#[test]
fn shortcut_is_not_presented_as_registered() {
    let view = ControlView::from_model(&selected_model());

    assert_eq!(view.shortcut_status, ShortcutDisplay::Released);
}

#[test]
fn checkbox_changes_save_immediately() {
    let temp = tempfile::tempdir().unwrap();
    let store_path = temp.path().join("overcrow/settings.json");
    let mut controller = ControlController::new(
        model_with(LifecycleSettings::default(), vec![game(620, "Portal 2")]),
        SettingsStore::from_path(&store_path),
    );

    assert_eq!(controller.set_steam_selected(620, true), SaveOutcome::Saved);
    let saved = SettingsStore::from_path(store_path).load().settings;
    assert!(saved.selected_steam_app_ids.contains(&620));
    assert!(!saved.enabled);
    assert!(controller.notice().is_none());
}

#[test]
fn checkbox_change_rolls_back_and_reports_non_modal_error_when_save_fails() {
    let temp = tempfile::tempdir().unwrap();
    let blocked_parent = temp.path().join("blocked");
    fs::write(&blocked_parent, b"not a directory").unwrap();
    let mut controller = ControlController::new(
        model_with(LifecycleSettings::default(), vec![game(620, "Portal 2")]),
        SettingsStore::from_path(blocked_parent.join("settings.json")),
    );

    assert_eq!(
        controller.set_steam_selected(620, true),
        SaveOutcome::RolledBack
    );
    assert!(
        !controller
            .model()
            .settings
            .selected_steam_app_ids
            .contains(&620)
    );
    let notice = controller
        .notice()
        .expect("save failure must remain visible");
    assert_eq!(notice.level, UiNoticeLevel::Error);
    assert!(!notice.modal);
    assert!(notice.message.contains("Could not save game selection"));
}

#[test]
fn committed_checkbox_change_is_retained_with_a_durability_warning() {
    let mut controller = controlled_controller(
        model_with(LifecycleSettings::default(), vec![game(620, "Portal 2")]),
        StoreBehavior::FailAfterCommit,
    );

    assert_eq!(
        controller.set_steam_selected(620, true),
        SaveOutcome::CommittedWithWarning
    );
    assert!(
        controller
            .model()
            .settings
            .selected_steam_app_ids
            .contains(&620)
    );
    let notice = controller
        .notice()
        .expect("durability warning must remain visible");
    assert_eq!(notice.operation, NoticeOperation::SelectionSave);
    assert_eq!(notice.level, UiNoticeLevel::Warning);
    assert!(notice.message.contains("applied"));
    assert!(notice.message.contains("durability"));
}

#[test]
fn committed_native_add_is_retained_with_a_warning() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("native-game");
    fs::write(&executable, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let mut controller = controlled_controller(
        model_with(LifecycleSettings::default(), Vec::new()),
        StoreBehavior::FailAfterCommit,
    );

    assert_eq!(
        controller.handle_picker_result(PickerResult::Selected(executable)),
        SaveOutcome::CommittedWithWarning
    );
    assert_eq!(controller.model().settings.manual_games.len(), 1);
}

#[test]
fn committed_native_removal_stays_removed_with_a_warning() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("native-game");
    fs::write(&executable, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let mut model = model_with(LifecycleSettings::default(), Vec::new());
    let id = model.add_manual_game("Native game", &executable).unwrap();
    let mut controller = controlled_controller(model, StoreBehavior::FailAfterCommit);

    assert_eq!(
        controller.remove_manual_game(&id),
        SaveOutcome::CommittedWithWarning
    );
    assert!(controller.model().settings.manual_games.is_empty());
}

#[test]
fn pre_commit_native_add_rolls_back() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("native-game");
    fs::write(&executable, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let mut controller = controlled_controller(
        model_with(LifecycleSettings::default(), Vec::new()),
        StoreBehavior::FailBeforeCommit,
    );

    assert_eq!(
        controller.handle_picker_result(PickerResult::Selected(executable)),
        SaveOutcome::RolledBack
    );
    assert!(controller.model().settings.manual_games.is_empty());
}

#[test]
fn refresh_is_off_thread_bounded_and_single_flight() {
    let temp = tempfile::tempdir().unwrap();
    let mut controller = controller(&temp, model_with(LifecycleSettings::default(), Vec::new()));
    let ui_thread = thread::current().id();
    let (worker_id_tx, worker_id_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(0);

    assert_eq!(
        controller.start_refresh_with(move || {
            worker_id_tx.send(thread::current().id()).unwrap();
            release_rx.recv().unwrap();
            DiscoveryReport::default()
        }),
        WorkStart::Started
    );
    assert!(controller.refresh_in_flight());
    assert_eq!(
        controller.start_refresh_with(|| panic!("duplicate refresh must not launch")),
        WorkStart::AlreadyRunning
    );
    let worker_id = worker_id_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_ne!(worker_id, ui_thread);

    release_tx.send(()).unwrap();
    poll_until(Duration::from_secs(2), || controller.poll_refresh());
    assert!(!controller.refresh_in_flight());
}

#[test]
fn refresh_result_replaces_discovery_without_changing_selections() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = LifecycleSettings::default();
    settings.selected_steam_app_ids.insert(620);
    let mut controller = controller(&temp, model_with(settings, vec![game(620, "Portal 2")]));

    assert_eq!(
        controller.start_refresh_with(|| DiscoveryReport {
            games: vec![game(1_623_730, "Palworld")],
            warnings: vec!["partial discovery".to_owned()],
        }),
        WorkStart::Started
    );
    poll_until(Duration::from_secs(2), || controller.poll_refresh());

    assert_eq!(
        controller
            .model()
            .games
            .iter()
            .map(|game| game.app_id)
            .collect::<Vec<_>>(),
        vec![1_623_730]
    );
    assert_eq!(controller.model().discovery_warnings, ["partial discovery"]);
    assert!(
        controller
            .model()
            .settings
            .selected_steam_app_ids
            .contains(&620)
    );
}

#[test]
fn cached_diagnostics_follow_selection_and_refresh_without_live_probes() {
    let temp = tempfile::tempdir().unwrap();
    let model = model_with(LifecycleSettings::default(), vec![game(620, "Portal 2")]);
    let mut controller = ControlController::new_with_diagnostic_input(
        model,
        SettingsStore::from_path(temp.path().join("overcrow/settings.json")),
        DiagnosticInput {
            session_type: Some("x11".into()),
            home: Some(PathBuf::from("/injected/home")),
            ..DiagnosticInput::default()
        },
    );

    let find = |controller: &ControlController, label: &str| {
        controller
            .diagnostic_report()
            .items
            .iter()
            .find(|item| item.label == label)
            .cloned()
            .unwrap()
    };
    assert!(find(&controller, "Desktop session").detail.contains("X11"));
    assert!(
        find(&controller, "Settings path")
            .detail
            .contains("/injected/home")
    );
    assert!(
        find(&controller, "Game selections")
            .detail
            .contains("0 Steam")
    );

    assert_eq!(controller.set_steam_selected(620, true), SaveOutcome::Saved);
    assert!(
        find(&controller, "Game selections")
            .detail
            .contains("1 Steam")
    );

    assert_eq!(
        controller.start_refresh_with(|| DiscoveryReport {
            games: vec![game(620, "Portal 2"), game(1_623_730, "Palworld")],
            warnings: vec!["partial discovery".into()],
        }),
        WorkStart::Started
    );
    poll_until(Duration::from_secs(2), || controller.poll_refresh());

    assert_eq!(find(&controller, "Steam discovery").level, Level::Ok);
    assert!(
        find(&controller, "Steam discovery")
            .detail
            .contains("2 Steam games")
    );
    assert!(
        find(&controller, "Steam discovery warning")
            .detail
            .contains("partial discovery")
    );
    assert!(find(&controller, "Desktop session").detail.contains("X11"));
    assert_eq!(controller.diagnostic_report().lifecycle_state, "Disabled");
}

#[test]
fn controller_normalizes_cached_diagnostics_across_repeated_refreshes() {
    let huge = "é".repeat(MAX_DIAGNOSTIC_DETAIL_BYTES * 4);
    let model = ControlModel::new(
        SettingsLoad {
            settings: LifecycleSettings::default(),
            warning: Some(huge.clone()),
        },
        DiscoveryReport {
            games: vec![game(620, "Portal 2")],
            warnings: vec![huge.clone(); MAX_DISCOVERY_WARNINGS + 8],
        },
        NativePathValidator,
    );
    let mut controller = ControlController::new_with_diagnostic_input(
        model,
        ControlledStore(StoreBehavior::Save),
        DiagnosticInput {
            session_type: Some(huge.clone()),
            current_desktop: Some(huge.clone()),
            desktop_session: Some(huge.clone()),
            home: Some(PathBuf::from(format!("/{}", "h".repeat(1_024)))),
            xdg_config_home: Some(PathBuf::from(format!("/{}", "x".repeat(1_024)))),
            settings_warning: Some(huge.clone()),
            discovery_warnings: vec![huge.clone(); MAX_DISCOVERY_WARNINGS + 8],
            ..DiagnosticInput::default()
        },
    );

    let assert_bounded = |controller: &ControlController| {
        let input = controller.diagnostic_input_for_test();
        assert!(input.session_type.as_ref().unwrap().len() <= MAX_SESSION_TYPE_BYTES);
        assert!(input.current_desktop.as_ref().unwrap().len() <= MAX_DESKTOP_METADATA_BYTES);
        assert!(input.desktop_session.as_ref().unwrap().len() <= MAX_DESKTOP_METADATA_BYTES);
        assert!(input.home.is_none());
        assert!(input.xdg_config_home.is_none());
        assert!(input.environment_was_truncated);
        assert!(input.model_was_truncated);
        assert!(input.settings_warning.as_ref().unwrap().len() <= MAX_WARNING_BYTES);
        assert!(input.discovery_warnings.len() <= MAX_DISCOVERY_WARNINGS);
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

        let report = controller.diagnostic_report();
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
    };

    assert_bounded(&controller);
    assert_eq!(controller.set_steam_selected(620, true), SaveOutcome::Saved);
    assert_bounded(&controller);

    for _ in 0..3 {
        let refresh_warning = huge.clone();
        assert_eq!(
            controller.start_refresh_with(move || DiscoveryReport {
                games: vec![game(620, "Portal 2")],
                warnings: vec![refresh_warning; MAX_DISCOVERY_WARNINGS + 8],
            }),
            WorkStart::Started
        );
        poll_until(Duration::from_secs(2), || controller.poll_refresh());
        assert_bounded(&controller);
    }
}

#[test]
fn selected_native_game_is_validated_by_the_real_validator_and_saved() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("native-game");
    fs::write(&executable, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let mut controller = controller(&temp, model_with(LifecycleSettings::default(), Vec::new()));

    assert_eq!(
        controller.handle_picker_result(PickerResult::Selected(executable.canonicalize().unwrap())),
        SaveOutcome::Saved
    );
    assert_eq!(controller.model().settings.manual_games.len(), 1);
    assert_eq!(
        controller.model().settings.manual_games[0].name,
        "native-game"
    );
    assert!(controller.notice().is_none());
}

#[test]
fn cancelled_native_picker_is_a_no_op_without_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let mut controller = controller(&temp, model_with(LifecycleSettings::default(), Vec::new()));

    assert_eq!(
        controller.handle_picker_result(PickerResult::CancelledOrUnavailable),
        SaveOutcome::Unchanged
    );
    assert!(controller.model().settings.manual_games.is_empty());
    assert!(controller.notice().is_none());
}

#[test]
fn invalid_native_picker_result_is_non_modal_and_does_not_mutate_settings() {
    let temp = tempfile::tempdir().unwrap();
    let not_executable = temp.path().join("not-executable");
    fs::write(&not_executable, b"data").unwrap();
    fs::set_permissions(&not_executable, fs::Permissions::from_mode(0o644)).unwrap();
    let mut controller = controller(&temp, model_with(LifecycleSettings::default(), Vec::new()));

    assert_eq!(
        controller.handle_picker_result(PickerResult::Selected(not_executable)),
        SaveOutcome::Unchanged
    );
    assert!(controller.model().settings.manual_games.is_empty());
    let notice = controller
        .notice()
        .expect("validation error must be visible");
    assert_eq!(notice.level, UiNoticeLevel::Error);
    assert!(!notice.modal);
    assert!(notice.message.contains("Could not add native game"));
}

#[test]
fn picker_worker_errors_are_non_modal_and_leave_settings_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let mut controller = controller(&temp, model_with(LifecycleSettings::default(), Vec::new()));

    assert_eq!(
        controller.handle_picker_result(PickerResult::Failed(PickerFailure::Worker(
            "portal helper unavailable".to_owned(),
        ))),
        SaveOutcome::Unchanged
    );
    assert!(controller.model().settings.manual_games.is_empty());
    let notice = controller.notice().expect("picker error must be visible");
    assert_eq!(notice.level, UiNoticeLevel::Error);
    assert!(!notice.modal);
    assert!(notice.message.contains("portal helper unavailable"));
}

#[test]
fn concurrent_operation_errors_coexist_and_cancellation_clears_only_picker_notice() {
    let mut controller = controlled_controller(
        model_with(LifecycleSettings::default(), vec![game(620, "Portal 2")]),
        StoreBehavior::FailBeforeCommit,
    );
    assert_eq!(
        controller.set_steam_selected(620, true),
        SaveOutcome::RolledBack
    );
    controller.handle_picker_result(PickerResult::Failed(PickerFailure::Worker(
        "portal helper failed".to_owned(),
    )));
    controller.start_refresh_with(|| panic!("refresh panic"));
    poll_until(Duration::from_secs(2), || controller.poll_refresh());

    let operations = controller
        .notices()
        .map(|notice| notice.operation)
        .collect::<Vec<_>>();
    assert_eq!(
        operations,
        [
            NoticeOperation::SelectionSave,
            NoticeOperation::Refresh,
            NoticeOperation::Picker,
        ]
    );

    controller.handle_picker_result(PickerResult::CancelledOrUnavailable);
    let operations = controller
        .notices()
        .map(|notice| notice.operation)
        .collect::<Vec<_>>();
    assert_eq!(
        operations,
        [NoticeOperation::SelectionSave, NoticeOperation::Refresh]
    );
}

#[test]
fn default_native_name_keeps_the_full_appimage_filename() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("Portal.AppImage");
    fs::write(&executable, b"native image").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let mut controller = controlled_controller(
        model_with(LifecycleSettings::default(), Vec::new()),
        StoreBehavior::Save,
    );

    assert_eq!(
        controller.handle_picker_result(PickerResult::Selected(executable)),
        SaveOutcome::Saved
    );
    assert_eq!(
        controller.model().settings.manual_games[0].name,
        "Portal.AppImage"
    );
}

#[test]
fn direct_picker_converts_selection_and_cancellation() {
    let selected = PathBuf::from("/games/Portal.AppImage");

    assert_eq!(
        picker_result(Some(selected.clone())),
        PickerResult::Selected(selected)
    );
    assert_eq!(picker_result(None), PickerResult::CancelledOrUnavailable);
}

#[test]
fn dropping_controller_does_not_wait_for_an_open_native_dialog() {
    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let temp = tempfile::tempdir().unwrap();
        let mut controller =
            controller(&temp, model_with(LifecycleSettings::default(), Vec::new()));
        assert_eq!(
            controller.start_picker_with(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                PickerResult::CancelledOrUnavailable
            }),
            WorkStart::Started
        );
        drop(controller);
        dropped_tx.send(()).unwrap();
    });

    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let dropped_without_waiting = dropped_rx.recv_timeout(Duration::from_millis(50)).is_ok();
    release_tx.send(()).unwrap();
    handle.join().unwrap();

    assert!(
        dropped_without_waiting,
        "controller drop waited for the native dialog worker"
    );
}
