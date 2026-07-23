use std::{
    any::Any,
    env,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError, sync_channel},
    },
    thread,
};

use overcrow_config::{SettingsStore, settings_save_was_committed};

use crate::diagnostics::collect_normalized_foundation_diagnostics;
use crate::{
    CompatibilityReport, ControlCompatibility, ControlDiagnostic, ControlGame, ControlLifecycle,
    ControlManualGame, ControlModel, ControlNotice, ControlOperationState, ControlSnapshot,
    DiagnosticInput, DiagnosticLevelCode, DiagnosticReport, DiscoveryReport, EnvironmentIdentity,
    LifecycleController, LifecycleSnapshot, LifecycleStatus, NativePathValidator, NoticeLevelCode,
    NoticeOperationCode, SelectionStore, candidate_steam_roots, discover_from_roots,
};

pub const APPLICATION_ID: &str = "com.playervox.OverCrow";
pub const APPLICATION_TITLE: &str = "PlayerVox OverCrow";
const WORKER_CHANNEL_CAPACITY: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppTab {
    Games,
    Settings,
    Diagnostics,
    About,
}

impl AppTab {
    const ALL: [Self; 4] = [Self::Games, Self::Settings, Self::Diagnostics, Self::About];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutDisplay {
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlView {
    pub tabs: [AppTab; 4],
    pub master_switch_enabled: bool,
    pub master_switch_checked: bool,
    pub lifecycle_label: &'static str,
    pub shortcut_status: ShortcutDisplay,
}

impl ControlView {
    pub fn from_model(model: &ControlModel) -> Self {
        let configured = !model.settings.selected_steam_app_ids.is_empty()
            || !model.settings.manual_games.is_empty();
        Self {
            tabs: AppTab::ALL,
            master_switch_enabled: false,
            master_switch_checked: false,
            lifecycle_label: if configured {
                "Configured — activation not installed"
            } else {
                "Not configured — activation not installed"
            },
            shortcut_status: ShortcutDisplay::Released,
        }
    }

    fn from_lifecycle(
        model: &ControlModel,
        status: LifecycleStatus,
        available: bool,
        transition: Option<LifecycleTransition>,
    ) -> Self {
        let configured = !model.settings.selected_steam_app_ids.is_empty()
            || !model.settings.manual_games.is_empty();
        let checked = matches!(status, LifecycleStatus::Enabled | LifecycleStatus::Warning)
            || matches!(transition, Some(LifecycleTransition::Enabling));
        Self {
            tabs: AppTab::ALL,
            master_switch_enabled: available && transition.is_none() && (checked || configured),
            master_switch_checked: checked,
            lifecycle_label: match transition {
                Some(LifecycleTransition::Enabling) => "Enabling…",
                Some(LifecycleTransition::Disabling) => "Disabling…",
                None => status.label(),
            },
            shortcut_status: ShortcutDisplay::Released,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifecycleTransition {
    Enabling,
    Disabling,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveOutcome {
    Saved,
    CommittedWithWarning,
    RolledBack,
    Unchanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkStart {
    Started,
    AlreadyRunning,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PickerResult {
    Selected(PathBuf),
    CancelledOrUnavailable,
    Failed(PickerFailure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PickerFailure {
    Worker(String),
}

impl std::fmt::Display for PickerFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Worker(error) => write!(formatter, "picker worker failed: {error}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiNoticeLevel {
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoticeOperation {
    SelectionSave,
    Refresh,
    Picker,
    Lifecycle,
}

impl NoticeOperation {
    const COUNT: usize = 4;

    const fn index(self) -> usize {
        match self {
            Self::SelectionSave => 0,
            Self::Refresh => 1,
            Self::Picker => 2,
            Self::Lifecycle => 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiNotice {
    pub operation: NoticeOperation,
    pub message: String,
    pub level: UiNoticeLevel,
    pub modal: bool,
}

type WorkerReceiver<T> = Receiver<Result<T, String>>;

struct PickerWorker {
    receiver: WorkerReceiver<PickerResult>,
    handle: Option<thread::JoinHandle<()>>,
    result: Option<Result<PickerResult, String>>,
}

struct LifecycleCompletion {
    result: Result<(), String>,
    snapshot: LifecycleSnapshot,
}

struct LifecycleWorker {
    transition: LifecycleTransition,
    receiver: WorkerReceiver<LifecycleCompletion>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LifecycleWorker {
    fn try_complete(&mut self) -> Option<Result<LifecycleCompletion, String>> {
        let result = match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("lifecycle worker stopped without a result".to_owned()))
            }
        }?;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        Some(result)
    }

    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl PickerWorker {
    fn try_complete(&mut self) -> Option<Result<PickerResult, String>> {
        if self.result.is_none() {
            self.result = match self.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err("native game picker stopped without a result".to_owned()))
                }
            };
        }
        if self.result.is_some()
            && self
                .handle
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished)
        {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
            return self.result.take();
        }
        None
    }
}

pub struct ControlController {
    model: ControlModel,
    store: Box<dyn SelectionStore>,
    diagnostic_input: DiagnosticInput,
    diagnostic_report: DiagnosticReport,
    notices: [Option<UiNotice>; NoticeOperation::COUNT],
    refresh_receiver: Option<WorkerReceiver<DiscoveryReport>>,
    picker_worker: Option<PickerWorker>,
    lifecycle: Option<Arc<LifecycleController>>,
    lifecycle_status: LifecycleStatus,
    lifecycle_worker: Option<LifecycleWorker>,
    compatibility: CompatibilityReport,
}

impl ControlController {
    pub fn production() -> Self {
        let store = SettingsStore::from_environment();
        let settings = store.load();
        let model = ControlModel::new(settings, DiscoveryReport::default(), NativePathValidator);
        let diagnostic_input = DiagnosticInput::from_current_process();
        let lifecycle = Arc::new(LifecycleController::production(
            SettingsStore::from_environment(),
        ));
        let mut controller = Self::new_with_lifecycle(model, store, diagnostic_input, lifecycle);
        controller.compatibility =
            CompatibilityReport::from_environment(EnvironmentIdentity::from_current_process());
        controller
    }

    pub fn new<S>(model: ControlModel, store: S) -> Self
    where
        S: SelectionStore + 'static,
    {
        Self::new_with_diagnostic_input(model, store, DiagnosticInput::default())
    }

    pub fn new_with_diagnostic_input<S>(
        model: ControlModel,
        store: S,
        diagnostic_input: DiagnosticInput,
    ) -> Self
    where
        S: SelectionStore + 'static,
    {
        Self::new_with_optional_lifecycle(model, store, diagnostic_input, None)
    }

    pub fn new_with_lifecycle<S>(
        model: ControlModel,
        store: S,
        diagnostic_input: DiagnosticInput,
        lifecycle: Arc<LifecycleController>,
    ) -> Self
    where
        S: SelectionStore + 'static,
    {
        Self::new_with_optional_lifecycle(model, store, diagnostic_input, Some(lifecycle))
    }

    fn new_with_optional_lifecycle<S>(
        model: ControlModel,
        store: S,
        diagnostic_input: DiagnosticInput,
        lifecycle: Option<Arc<LifecycleController>>,
    ) -> Self
    where
        S: SelectionStore + 'static,
    {
        let mut diagnostic_input = diagnostic_input.normalize();
        let compatibility = CompatibilityReport::from_environment(EnvironmentIdentity {
            session_type: diagnostic_input.session_type.clone(),
            current_desktop: diagnostic_input.current_desktop.clone(),
            desktop_session: diagnostic_input.desktop_session.clone(),
            os_name: None,
        });
        diagnostic_input.sync_model(&model);
        let lifecycle_status = lifecycle.as_ref().map_or_else(
            || {
                if model.settings.enabled {
                    LifecycleStatus::Enabled
                } else {
                    LifecycleStatus::Disabled
                }
            },
            |controller| controller.status(),
        );
        diagnostic_input.set_lifecycle_status(lifecycle_status);
        let diagnostic_report = collect_normalized_foundation_diagnostics(&diagnostic_input);
        Self {
            model,
            store: Box::new(store),
            diagnostic_input,
            diagnostic_report,
            notices: std::array::from_fn(|_| None),
            refresh_receiver: None,
            picker_worker: None,
            lifecycle,
            lifecycle_status,
            lifecycle_worker: None,
            compatibility,
        }
    }

    pub fn model(&self) -> &ControlModel {
        &self.model
    }

    pub fn view(&self) -> ControlView {
        ControlView::from_lifecycle(
            &self.model,
            self.lifecycle_status,
            self.lifecycle.is_some(),
            self.lifecycle_worker
                .as_ref()
                .map(|worker| worker.transition),
        )
    }

    pub fn diagnostic_report(&self) -> &DiagnosticReport {
        &self.diagnostic_report
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_input_for_test(&self) -> &DiagnosticInput {
        &self.diagnostic_input
    }

    pub fn notice(&self) -> Option<&UiNotice> {
        self.notices().next()
    }

    pub fn notices(&self) -> impl Iterator<Item = &UiNotice> {
        self.notices.iter().filter_map(Option::as_ref)
    }

    pub fn snapshot(&self) -> ControlSnapshot {
        let view = self.view();
        ControlSnapshot {
            schema_version: crate::CONTROL_SNAPSHOT_SCHEMA_VERSION,
            compatibility: ControlCompatibility::from(&self.compatibility),
            lifecycle: ControlLifecycle::from_status_and_label(
                self.lifecycle_status,
                view.lifecycle_label,
            ),
            master_switch_enabled: view.master_switch_enabled
                && self.compatibility.activation_allowed,
            master_switch_checked: view.master_switch_checked,
            selection_editing_enabled: self.selection_editing_enabled(),
            shortcut: crate::presentation::bounded_control_text(
                &self.model.settings.shortcut.accelerator,
                crate::presentation::MAX_CONTROL_SHORTCUT_BYTES,
            ),
            operations: ControlOperationState {
                refresh: self.refresh_in_flight(),
                picker: self.picker_in_flight(),
                lifecycle: self.lifecycle_in_flight(),
            },
            games: self
                .model
                .games
                .iter()
                .take(crate::presentation::MAX_CONTROL_GAMES)
                .map(|game| ControlGame {
                    app_id: game.app_id,
                    name: crate::presentation::bounded_control_text(
                        &game.name,
                        crate::MAX_CONTROL_GAME_NAME_BYTES,
                    ),
                    selected: self
                        .model
                        .settings
                        .selected_steam_app_ids
                        .contains(&game.app_id),
                })
                .collect(),
            manual_games: self
                .model
                .settings
                .manual_games
                .iter()
                .take(crate::presentation::MAX_CONTROL_MANUAL_GAMES)
                .map(|game| ControlManualGame {
                    id: crate::presentation::bounded_control_text(
                        &game.id,
                        crate::presentation::MAX_CONTROL_ID_BYTES,
                    ),
                    name: crate::presentation::bounded_control_text(
                        &game.name,
                        crate::MAX_CONTROL_GAME_NAME_BYTES,
                    ),
                    executable: crate::presentation::bounded_control_text(
                        game.executable.to_str().unwrap_or("Unavailable"),
                        crate::presentation::MAX_CONTROL_PATH_BYTES,
                    ),
                })
                .collect(),
            notices: self
                .notices()
                .map(|notice| ControlNotice {
                    operation: NoticeOperationCode::from(notice.operation),
                    level: NoticeLevelCode::from(notice.level),
                    message: crate::presentation::bounded_control_text(
                        &notice.message,
                        crate::presentation::MAX_CONTROL_MESSAGE_BYTES,
                    ),
                })
                .collect(),
            diagnostics: self
                .diagnostic_report
                .items
                .iter()
                .take(crate::presentation::MAX_CONTROL_DIAGNOSTICS)
                .map(|item| ControlDiagnostic {
                    label: crate::presentation::bounded_control_text(
                        &item.label,
                        crate::presentation::MAX_CONTROL_DIAGNOSTIC_LABEL_BYTES,
                    ),
                    detail: crate::presentation::bounded_control_text(
                        &item.detail,
                        crate::presentation::MAX_CONTROL_MESSAGE_BYTES,
                    ),
                    level: DiagnosticLevelCode::from(item.level),
                })
                .collect(),
        }
    }

    pub fn poll_pending(&mut self) -> bool {
        self.poll_refresh() | self.poll_picker() | self.poll_lifecycle()
    }

    pub fn request_master_toggle(&mut self, requested: bool) -> bool {
        if self.lifecycle_worker.is_some() {
            return false;
        }
        let Some(lifecycle) = self.lifecycle.clone() else {
            return false;
        };
        if requested && self.lifecycle_status == LifecycleStatus::Warning {
            self.set_error(
                NoticeOperation::Lifecycle,
                "Clean up the settings warning before enabling OverCrow".to_owned(),
            );
            return false;
        }
        if requested
            && self.model.settings.selected_steam_app_ids.is_empty()
            && self.model.settings.manual_games.is_empty()
        {
            self.set_error(
                NoticeOperation::Lifecycle,
                "Select at least one game before enabling OverCrow".to_owned(),
            );
            return false;
        }
        let settings = self.model.settings.clone();
        let transition = if requested {
            LifecycleTransition::Enabling
        } else {
            LifecycleTransition::Disabling
        };
        let (sender, receiver) = sync_channel(WORKER_CHANNEL_CAPACITY);
        let worker = thread::Builder::new()
            .name("overcrow-lifecycle".to_owned())
            .spawn(move || {
                let operation = if requested {
                    lifecycle.enable(&settings)
                } else {
                    lifecycle.disable()
                };
                let completion = LifecycleCompletion {
                    result: operation.map_err(|error| error.to_string()),
                    snapshot: lifecycle.snapshot(),
                };
                let _ = sender.send(Ok(completion));
            });
        let worker = match worker {
            Ok(handle) => LifecycleWorker {
                transition,
                receiver,
                handle: Some(handle),
            },
            Err(error) => {
                self.set_error(
                    NoticeOperation::Lifecycle,
                    format!("Could not start lifecycle transaction: {error}"),
                );
                return false;
            }
        };
        self.clear_notice(NoticeOperation::Lifecycle);
        self.lifecycle_worker = Some(worker);
        true
    }

    pub fn lifecycle_in_flight(&self) -> bool {
        self.lifecycle_worker.is_some()
    }

    pub fn selection_editing_enabled(&self) -> bool {
        self.lifecycle_worker.is_none() && self.lifecycle_status == LifecycleStatus::Disabled
    }

    pub fn poll_lifecycle(&mut self) -> bool {
        let received = match self.lifecycle_worker.as_mut() {
            Some(worker) => worker.try_complete(),
            None => return false,
        };
        let Some(received) = received else {
            return false;
        };
        self.lifecycle_worker = None;
        match received {
            Ok(completion) => {
                self.model.apply_settings_load(completion.snapshot.load);
                self.lifecycle_status = completion.snapshot.status;
                match completion.result {
                    Ok(()) => self.clear_notice(NoticeOperation::Lifecycle),
                    Err(error) => self.set_error(
                        NoticeOperation::Lifecycle,
                        format!("Lifecycle transaction failed: {error}"),
                    ),
                }
            }
            Err(error) => self.set_error(NoticeOperation::Lifecycle, error),
        }
        self.refresh_diagnostics();
        true
    }

    pub fn set_search(&mut self, search: &str) {
        self.model.set_search(search);
    }

    pub fn set_steam_selected(&mut self, app_id: u32, selected: bool) -> SaveOutcome {
        self.selection_transaction(|model| {
            let was_selected = model.settings.selected_steam_app_ids.contains(&app_id);
            model.set_steam_selected(app_id, selected);
            let is_selected = model.settings.selected_steam_app_ids.contains(&app_id);
            Ok::<bool, std::convert::Infallible>(was_selected != is_selected)
        })
        .unwrap_or_else(|never| match never {})
    }

    pub fn remove_manual_game(&mut self, id: &str) -> SaveOutcome {
        self.selection_transaction(|model| {
            Ok::<bool, std::convert::Infallible>(model.remove_manual_game(id))
        })
        .unwrap_or_else(|never| match never {})
    }

    pub fn start_refresh(&mut self) -> WorkStart {
        self.start_refresh_with(|| {
            let roots = home_directory()
                .map(|home| candidate_steam_roots(&home))
                .unwrap_or_default();
            discover_from_roots(&roots)
        })
    }

    pub fn start_refresh_with<F>(&mut self, refresh: F) -> WorkStart
    where
        F: FnOnce() -> DiscoveryReport + Send + 'static,
    {
        if self.refresh_receiver.is_some() {
            return WorkStart::AlreadyRunning;
        }

        match spawn_bounded_worker("overcrow-discovery", refresh) {
            Ok(receiver) => {
                self.clear_notice(NoticeOperation::Refresh);
                self.refresh_receiver = Some(receiver);
                WorkStart::Started
            }
            Err(error) => {
                self.set_error(
                    NoticeOperation::Refresh,
                    format!("Could not start game discovery: {error}"),
                );
                WorkStart::Failed
            }
        }
    }

    pub fn refresh_in_flight(&self) -> bool {
        self.refresh_receiver.is_some()
    }

    pub fn poll_refresh(&mut self) -> bool {
        let received = match self.refresh_receiver.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return false,
        };
        match received {
            Ok(Ok(report)) => {
                self.refresh_receiver = None;
                self.model.games = report.games;
                self.model.discovery_warnings = report.warnings;
                self.clear_notice(NoticeOperation::Refresh);
                self.refresh_diagnostics();
                true
            }
            Ok(Err(error)) => {
                self.refresh_receiver = None;
                self.set_error(
                    NoticeOperation::Refresh,
                    format!("Game discovery failed: {error}"),
                );
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.refresh_receiver = None;
                self.set_error(
                    NoticeOperation::Refresh,
                    "Game discovery worker stopped without a result".to_owned(),
                );
                true
            }
        }
    }

    pub fn start_native_picker(&mut self) -> WorkStart {
        self.start_picker_with(|| {
            let selected = pollster::block_on(
                rfd::AsyncFileDialog::new()
                    .set_title("Select a native game executable")
                    .pick_file(),
            );
            picker_result(selected.map(|file| file.path().to_path_buf()))
        })
    }

    pub fn start_picker_with<F>(&mut self, picker: F) -> WorkStart
    where
        F: FnOnce() -> PickerResult + Send + 'static,
    {
        if self.picker_worker.is_some() {
            return WorkStart::AlreadyRunning;
        }

        match spawn_picker_worker(picker) {
            Ok(worker) => {
                self.clear_notice(NoticeOperation::Picker);
                self.picker_worker = Some(worker);
                WorkStart::Started
            }
            Err(error) => {
                self.set_error(
                    NoticeOperation::Picker,
                    format!("Could not start native game picker: {error}"),
                );
                WorkStart::Failed
            }
        }
    }

    pub fn picker_in_flight(&self) -> bool {
        self.picker_worker.is_some()
    }

    pub fn poll_picker(&mut self) -> bool {
        let received = match self.picker_worker.as_mut() {
            Some(worker) => worker.try_complete(),
            None => return false,
        };
        match received {
            Some(Ok(result)) => {
                self.picker_worker = None;
                self.handle_picker_result(result);
                true
            }
            Some(Err(error)) => {
                self.picker_worker = None;
                self.handle_picker_result(PickerResult::Failed(PickerFailure::Worker(error)));
                true
            }
            None => false,
        }
    }

    pub fn handle_picker_result(&mut self, result: PickerResult) -> SaveOutcome {
        let path = match result {
            PickerResult::Selected(path) => path,
            PickerResult::CancelledOrUnavailable => {
                self.clear_notice(NoticeOperation::Picker);
                return SaveOutcome::Unchanged;
            }
            PickerResult::Failed(error) => {
                self.set_error(
                    NoticeOperation::Picker,
                    format!("Could not add native game: {error}"),
                );
                return SaveOutcome::Unchanged;
            }
        };
        self.clear_notice(NoticeOperation::Picker);
        let Some(name) = native_game_name(&path) else {
            self.set_error(
                NoticeOperation::Picker,
                "Could not add native game: executable name is unavailable".to_owned(),
            );
            return SaveOutcome::Unchanged;
        };

        match self.selection_transaction(|model| model.add_manual_game(&name, &path).map(|_| true))
        {
            Ok(outcome) => outcome,
            Err(error) => {
                self.set_error(
                    NoticeOperation::Picker,
                    format!("Could not add native game: {error}"),
                );
                SaveOutcome::Unchanged
            }
        }
    }

    fn selection_transaction<E, F>(&mut self, mutate: F) -> Result<SaveOutcome, E>
    where
        F: FnOnce(&mut ControlModel) -> Result<bool, E>,
    {
        if !self.selection_editing_enabled() {
            return Ok(SaveOutcome::Unchanged);
        }
        let prior = self.model.settings.clone();
        let changed = match mutate(&mut self.model) {
            Ok(changed) => changed,
            Err(error) => {
                self.model.settings = prior;
                self.refresh_diagnostics();
                return Err(error);
            }
        };
        if !changed {
            self.refresh_diagnostics();
            return Ok(SaveOutcome::Unchanged);
        }

        let save_result = self.model.save_selections(self.store.as_ref());
        let outcome = match save_result {
            Ok(()) => {
                self.clear_notice(NoticeOperation::SelectionSave);
                SaveOutcome::Saved
            }
            Err(error) if settings_save_was_committed(&error) => {
                self.set_notice(
                    NoticeOperation::SelectionSave,
                    UiNoticeLevel::Warning,
                    format!(
                        "Change was applied, but settings durability could not be confirmed: {error}"
                    ),
                );
                SaveOutcome::CommittedWithWarning
            }
            Err(error) => {
                self.model.settings = prior;
                self.set_error(
                    NoticeOperation::SelectionSave,
                    format!("Could not save game selection: {error}"),
                );
                SaveOutcome::RolledBack
            }
        };
        self.refresh_diagnostics();
        Ok(outcome)
    }

    fn set_error(&mut self, operation: NoticeOperation, message: String) {
        self.set_notice(operation, UiNoticeLevel::Error, message);
    }

    fn set_notice(&mut self, operation: NoticeOperation, level: UiNoticeLevel, message: String) {
        self.notices[operation.index()] = Some(UiNotice {
            operation,
            message,
            level,
            modal: false,
        });
    }

    fn clear_notice(&mut self, operation: NoticeOperation) {
        self.notices[operation.index()] = None;
    }

    fn refresh_diagnostics(&mut self) {
        self.diagnostic_input.sync_model(&self.model);
        self.diagnostic_input
            .set_lifecycle_status(self.lifecycle_status);
        self.diagnostic_report = collect_normalized_foundation_diagnostics(&self.diagnostic_input);
    }
}

impl Drop for ControlController {
    fn drop(&mut self) {
        if let Some(mut worker) = self.lifecycle_worker.take() {
            worker.join();
        }
    }
}

fn home_directory() -> Option<PathBuf> {
    let home = PathBuf::from(env::var_os("HOME")?);
    home.is_absolute().then_some(home)
}

fn native_game_name(path: &std::path::Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

pub(crate) fn picker_result(path: Option<PathBuf>) -> PickerResult {
    match path {
        Some(path) => PickerResult::Selected(path),
        None => PickerResult::CancelledOrUnavailable,
    }
}

fn spawn_bounded_worker<T, F>(name: &str, worker: F) -> Result<WorkerReceiver<T>, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = sync_channel(WORKER_CHANNEL_CAPACITY);
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(worker)).map_err(panic_message);
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;
    Ok(receiver)
}

fn spawn_picker_worker<F>(picker: F) -> Result<PickerWorker, String>
where
    F: FnOnce() -> PickerResult + Send + 'static,
{
    let (sender, receiver) = sync_channel(WORKER_CHANNEL_CAPACITY);
    let handle = thread::Builder::new()
        .name("overcrow-file-picker".to_owned())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(picker)).map_err(panic_message);
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;
    Ok(PickerWorker {
        receiver,
        handle: Some(handle),
        result: None,
    })
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "worker panicked".to_owned())
}
