use std::{
    env, fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use overcrow_config::{
    LifecycleSettings, ManualGame, SettingsLoad, SettingsStore, settings_save_was_committed,
};
use overcrow_protocol::Core1Proxy;

use crate::integration::{
    IntegrationController, IntegrationSetup, is_normal_absolute_path,
    is_trusted_system_control_binary, run_bounded_command,
};
use crate::model::{NativePathValidator, validate_stored_manual_game};

pub(crate) const SYSTEMCTL_PROGRAM: &str = "/usr/bin/systemctl";
pub(crate) const TIMEOUT_PROGRAM: &str = "/usr/bin/timeout";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const COMMAND_TIMEOUT_ARGUMENT: &str = "5s";
const COMMAND_KILL_AFTER_ARGUMENT: &str = "--kill-after=1s";
const MAX_SYSTEMCTL_OUTPUT_BYTES: u64 = 4096;
const CORE_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const SERVICE_UNKNOWN: &str = "org.freedesktop.DBus.Error.ServiceUnknown";
const ACTIVATION_UNITS: [&str; 3] = [
    "overcrow-core.service",
    "overcrow-overlay.service",
    "overcrow-hyprland.service",
];

pub(crate) const ENABLE_CORE: &[&str] = &["--user", "enable", "--now", "overcrow-core.service"];
pub(crate) const DISABLE_CORE: &[&str] = &["--user", "disable", "--now", "overcrow-core.service"];
pub(crate) const STOP_SESSION: &[&str] = &[
    "--user",
    "stop",
    "overcrow-hyprland.service",
    "overcrow-overlay.service",
];

pub trait LifecycleSettingsRepository: Send + Sync {
    fn load(&self) -> SettingsLoad;
    fn save(&self, settings: &LifecycleSettings) -> io::Result<()>;
}

impl LifecycleSettingsRepository for SettingsStore {
    fn load(&self) -> SettingsLoad {
        SettingsStore::load(self)
    }

    fn save(&self, settings: &LifecycleSettings) -> io::Result<()> {
        SettingsStore::save(self, settings)
    }
}

pub trait CommandRunner: Send + Sync {
    fn prepare_activation(&self) -> Result<(), String>;
    fn run_systemctl(&self, args: &'static [&'static str]) -> Result<(), String>;
}

pub struct SystemCommandRunner {
    unit_directory: Result<PathBuf, String>,
}

impl SystemCommandRunner {
    fn from_current_process() -> Self {
        let unit_directory = env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(|error| format!("could not identify the installed control binary: {error}"))
            .and_then(|executable| {
                let home = env::var_os("HOME").map(PathBuf::from);
                let data_home = env::var_os("XDG_DATA_HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from);
                supported_unit_directory(&executable, home.as_deref(), data_home.as_deref())
            });
        Self { unit_directory }
    }
}

impl CommandRunner for SystemCommandRunner {
    fn prepare_activation(&self) -> Result<(), String> {
        let unit_directory = self.unit_directory.as_ref().map_err(Clone::clone)?;
        prepare_activation_in(unit_directory, run_timed_systemctl)
    }

    fn run_systemctl(&self, args: &'static [&'static str]) -> Result<(), String> {
        run_bounded_command(SYSTEMCTL_PROGRAM, args, COMMAND_TIMEOUT)
    }
}

#[cfg(test)]
pub(crate) fn prepare_activation_with(
    executable: &Path,
    home: Option<&Path>,
    data_home: Option<&Path>,
    run_systemctl: impl FnMut(&[&str]) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let unit_directory = supported_unit_directory(executable, home, data_home)?;
    prepare_activation_in(&unit_directory, run_systemctl)
}

fn prepare_activation_in(
    unit_directory: &Path,
    mut run_systemctl: impl FnMut(&[&str]) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    run_systemctl(&["--user", "daemon-reload"])
        .map_err(|error| format!("systemd daemon-reload failed: {error}"))?;
    for unit in ACTIVATION_UNITS {
        let expected = unit_directory.join(unit);
        let output = run_systemctl(&["--user", "show", "--property=FragmentPath", "--value", unit])
            .map_err(|error| format!("could not query FragmentPath for {unit}: {error}"))?;
        if !exact_fragment_output(&output, &expected) {
            return Err(format!(
                "foreign or ambiguous FragmentPath for {unit}; expected {}",
                expected.display()
            ));
        }
    }
    Ok(())
}

pub(crate) fn supported_unit_directory(
    executable: &Path,
    home: Option<&Path>,
    data_home: Option<&Path>,
) -> Result<PathBuf, String> {
    if !is_normal_absolute_path(executable) {
        return Err("control binary path is not an absolute normalized path".to_owned());
    }
    if is_trusted_system_control_binary(executable) {
        return Ok(PathBuf::from("/usr/lib/systemd/user"));
    }

    let Some(home) = home.filter(|path| is_normal_absolute_path(path)) else {
        return Err("an absolute HOME is required for a local installation".to_owned());
    };
    if executable != home.join(".local/bin/overcrow-control") {
        return Err("control binary is not in a supported installed layout".to_owned());
    }
    let data_home = match data_home {
        Some(path) if is_normal_absolute_path(path) => path.to_path_buf(),
        Some(_) => return Err("XDG_DATA_HOME is not an absolute normalized path".to_owned()),
        None => home.join(".local/share"),
    };
    Ok(data_home.join("systemd/user"))
}

pub(crate) fn exact_fragment_output(output: &[u8], expected: &Path) -> bool {
    let expected = expected.as_os_str().as_encoded_bytes();
    output.len() == expected.len() + 1
        && output[..expected.len()] == *expected
        && output[expected.len()] == b'\n'
}

fn run_timed_systemctl(args: &[&str]) -> Result<Vec<u8>, String> {
    let mut child = Command::new(TIMEOUT_PROGRAM)
        .arg("--signal=TERM")
        .arg(COMMAND_KILL_AFTER_ARGUMENT)
        .arg(COMMAND_TIMEOUT_ARGUMENT)
        .arg(SYSTEMCTL_PROGRAM)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start bounded systemctl command: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "bounded systemctl stdout pipe is unavailable".to_owned())?;
    let mut output = Vec::new();
    let read_result = stdout
        .take(MAX_SYSTEMCTL_OUTPUT_BYTES + 1)
        .read_to_end(&mut output);
    let status = child
        .wait()
        .map_err(|error| format!("could not reap bounded systemctl command: {error}"))?;
    read_result.map_err(|error| format!("could not read bounded systemctl output: {error}"))?;
    if output.len() as u64 > MAX_SYSTEMCTL_OUTPUT_BYTES {
        return Err("systemctl output exceeded the byte limit".to_owned());
    }
    if !status.success() {
        return Err(format!("bounded systemctl command exited with {status}"));
    }
    Ok(output)
}

pub trait CorePassiveClient: Send + Sync {
    fn request_passive(&self) -> Result<(), String>;
}

pub trait LifecycleManualIdentityValidator: Send + Sync {
    fn validate(&self, game: &ManualGame) -> Result<(), String>;
}

#[derive(Default)]
pub struct NativeLifecycleManualIdentityValidator;

impl LifecycleManualIdentityValidator for NativeLifecycleManualIdentityValidator {
    fn validate(&self, game: &ManualGame) -> Result<(), String> {
        validate_stored_manual_game(&NativePathValidator, game)
    }
}

#[derive(Default)]
pub struct DbusCorePassiveClient;

impl CorePassiveClient for DbusCorePassiveClient {
    fn request_passive(&self) -> Result<(), String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("could not create Core request runtime: {error}"))?;
        runtime.block_on(async {
            let request = async {
                let connection = zbus::Connection::session().await?;
                let proxy = Core1Proxy::new(&connection).await?;
                proxy.set_overlay_interactive(false).await.map(|_| ())
            };
            match tokio::time::timeout(CORE_REQUEST_TIMEOUT, request).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) if is_service_unknown(&error) => Ok(()),
                Ok(Err(error)) => Err(format!("Core Passive request failed: {error}")),
                Err(_) => Err("Core Passive request timed out".to_owned()),
            }
        })
    }
}

fn is_service_unknown(error: &zbus::Error) -> bool {
    matches!(error, zbus::Error::MethodError(name, _, _) if is_service_unknown_name(name.as_str()))
}

pub(crate) fn is_service_unknown_name(name: &str) -> bool {
    name == SERVICE_UNKNOWN
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LifecycleStatus {
    #[default]
    Disabled,
    Enabled,
    Warning,
}

impl LifecycleStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Enabled => "Enabled",
            Self::Warning => "Cleanup required (settings warning)",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleError {
    primary: String,
    cleanup: Vec<String>,
}

impl LifecycleError {
    fn new(primary: impl Into<String>) -> Self {
        Self {
            primary: primary.into(),
            cleanup: Vec::new(),
        }
    }

    fn push_cleanup(&mut self, error: impl Into<String>) {
        self.cleanup.push(error.into());
    }
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.primary)?;
        for error in &self.cleanup {
            write!(formatter, "; cleanup failed: {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LifecycleError {}

pub struct LifecycleSnapshot {
    pub load: SettingsLoad,
    pub status: LifecycleStatus,
}

pub struct LifecycleController {
    transaction: Mutex<()>,
    repository: Arc<dyn LifecycleSettingsRepository>,
    integration: Arc<dyn IntegrationSetup>,
    commands: Arc<dyn CommandRunner>,
    passive: Arc<dyn CorePassiveClient>,
    manual_identity_validator: Arc<dyn LifecycleManualIdentityValidator>,
}

impl LifecycleController {
    pub fn production(store: SettingsStore) -> Self {
        let integration: Arc<dyn IntegrationSetup> =
            match IntegrationController::from_current_process() {
                Ok(controller) => Arc::new(controller),
                Err(error) => Arc::new(UnavailableIntegration(error)),
            };
        Self::injected(
            Arc::new(store),
            integration,
            Arc::new(SystemCommandRunner::from_current_process()),
            Arc::new(DbusCorePassiveClient),
        )
    }

    pub fn injected(
        repository: Arc<dyn LifecycleSettingsRepository>,
        integration: Arc<dyn IntegrationSetup>,
        commands: Arc<dyn CommandRunner>,
        passive: Arc<dyn CorePassiveClient>,
    ) -> Self {
        Self::injected_with_validator(
            repository,
            integration,
            commands,
            passive,
            Arc::new(NativeLifecycleManualIdentityValidator),
        )
    }

    pub fn injected_with_validator(
        repository: Arc<dyn LifecycleSettingsRepository>,
        integration: Arc<dyn IntegrationSetup>,
        commands: Arc<dyn CommandRunner>,
        passive: Arc<dyn CorePassiveClient>,
        manual_identity_validator: Arc<dyn LifecycleManualIdentityValidator>,
    ) -> Self {
        Self {
            transaction: Mutex::new(()),
            repository,
            integration,
            commands,
            passive,
            manual_identity_validator,
        }
    }

    pub fn enable(&self, settings: &LifecycleSettings) -> Result<(), LifecycleError> {
        let _guard = self
            .transaction
            .lock()
            .map_err(|_| LifecycleError::new("lifecycle transaction lock is unavailable"))?;
        let mut enabled = settings.clone();
        enabled.enabled = true;
        enabled = enabled
            .validate()
            .map_err(|error| LifecycleError::new(format!("invalid lifecycle settings: {error}")))?;
        if enabled.selected_steam_app_ids.is_empty() && enabled.manual_games.is_empty() {
            return Err(LifecycleError::new(
                "at least one exact game identity must be selected",
            ));
        }

        let current = self.repository.load();
        if let Some(warning) = current.warning {
            return Err(LifecycleError::new(format!(
                "cannot enable while lifecycle settings have a warning: {warning}"
            )));
        }
        for game in &enabled.manual_games {
            self.manual_identity_validator
                .validate(game)
                .map_err(|error| {
                    LifecycleError::new(format!("manual game identity is no longer valid: {error}"))
                })?;
        }

        self.commands.prepare_activation().map_err(|error| {
            LifecycleError::new(format!("systemd unit identity preflight failed: {error}"))
        })?;

        if let Err(error) = self.repository.save(&enabled) {
            return Err(self.rollback_after_enable(save_error(
                "could not persist enabled lifecycle settings",
                &error,
            )));
        }
        if let Err(error) = self.integration.ensure_ready() {
            return Err(self.rollback_after_enable(format!("integration setup failed: {error}")));
        }
        if let Err(error) = self.commands.run_systemctl(ENABLE_CORE) {
            return Err(self.rollback_after_enable(format!("could not enable Core: {error}")));
        }
        Ok(())
    }

    pub fn disable(&self) -> Result<(), LifecycleError> {
        let _guard = self
            .transaction
            .lock()
            .map_err(|_| LifecycleError::new("lifecycle transaction lock is unavailable"))?;
        let load = self.repository.load();
        let mut disabled = load.settings;
        disabled.enabled = false;

        let mut errors = Vec::new();
        match load.warning {
            Some(warning) => errors.push(format!(
                "settings load warning; problematic settings were preserved and the disable write was skipped: {warning}"
            )),
            None => {
                if let Err(error) = self.repository.save(&disabled) {
                    errors.push(save_error(
                        "could not persist disabled lifecycle settings",
                        &error,
                    ));
                }
            }
        }
        self.run_cleanup(&mut errors);

        errors_to_result(errors)
    }

    pub fn status(&self) -> LifecycleStatus {
        self.snapshot().status
    }

    pub fn snapshot(&self) -> LifecycleSnapshot {
        let _guard = match self.transaction.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return LifecycleSnapshot {
                    load: SettingsLoad {
                        settings: LifecycleSettings::default(),
                        warning: Some("lifecycle transaction lock is unavailable".to_owned()),
                    },
                    status: LifecycleStatus::Warning,
                };
            }
        };
        let load = self.repository.load();
        let status = if load.warning.is_some() {
            LifecycleStatus::Warning
        } else if load.settings.enabled {
            LifecycleStatus::Enabled
        } else {
            LifecycleStatus::Disabled
        };
        LifecycleSnapshot { load, status }
    }

    fn rollback_after_enable(&self, primary: String) -> LifecycleError {
        let load = self.repository.load();
        let mut disabled = load.settings;
        disabled.enabled = false;
        let mut error = LifecycleError::new(primary);
        match load.warning {
            Some(warning) => error.push_cleanup(format!(
                "settings load warning; problematic settings were preserved and the rollback write was skipped: {warning}"
            )),
            None => {
                if let Err(save) = self.repository.save(&disabled) {
                    error.push_cleanup(save_error(
                        "could not persist disabled rollback settings",
                        &save,
                    ));
                }
            }
        }
        let mut cleanup = Vec::new();
        self.run_cleanup(&mut cleanup);
        for failure in cleanup {
            error.push_cleanup(failure);
        }
        error
    }

    fn run_cleanup(&self, errors: &mut Vec<String>) {
        if let Err(error) = self.passive.request_passive() {
            errors.push(error);
        }
        if let Err(error) = self.commands.run_systemctl(STOP_SESSION) {
            errors.push(format!("could not stop session units: {error}"));
        }
        if let Err(error) = self.commands.run_systemctl(DISABLE_CORE) {
            errors.push(format!("could not disable Core: {error}"));
        }
    }
}

fn save_error(context: &str, error: &io::Error) -> String {
    if settings_save_was_committed(error) {
        format!("{context} (replacement committed but durability unknown): {error}")
    } else {
        format!("{context}: {error}")
    }
}

fn errors_to_result(mut errors: Vec<String>) -> Result<(), LifecycleError> {
    if errors.is_empty() {
        return Ok(());
    }
    let mut error = LifecycleError::new(errors.remove(0));
    for cleanup in errors {
        error.push_cleanup(cleanup);
    }
    Err(error)
}

struct UnavailableIntegration(String);

impl IntegrationSetup for UnavailableIntegration {
    fn ensure_ready(&self) -> Result<(), String> {
        Err(self.0.clone())
    }
}
