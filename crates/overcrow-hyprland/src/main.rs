use std::{ffi::OsString, path::PathBuf};

use anyhow::{Context, anyhow};
use overcrow_config::{SettingsLoad, SettingsStore};
use overcrow_hyprland::{
    bridge::{cleanup_runtime_state, run_bridge},
    ipc::{HyprlandIpc, SocketPaths},
    shortcut::ShortcutSpec,
};
use overcrow_logging::{Component, EventLogger, LoggerRuntime};
use overcrow_protocol::Core1Proxy;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Run,
    CleanupFocusState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupAction {
    Exit,
    Run,
    CleanupFocusState,
}

fn parse_mode(args: impl IntoIterator<Item = OsString>) -> Result<Mode, String> {
    let mut args = args.into_iter();
    match (args.next(), args.next()) {
        (None, None) => Ok(Mode::Run),
        (Some(flag), None) if flag == "--cleanup-focus-state" => Ok(Mode::CleanupFocusState),
        _ => Err("usage: overcrow-hyprland [--cleanup-focus-state]".to_owned()),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mode = parse_mode(std::env::args_os().skip(1)).map_err(|error| anyhow!(error))?;
    let settings_load = (mode == Mode::Run).then(|| SettingsStore::from_environment().load());
    if let Some(warning) = settings_load
        .as_ref()
        .and_then(|settings_load| settings_load.warning.as_ref())
    {
        eprintln!("OverCrow lifecycle settings rejected; remaining inert: {warning}");
    }

    let shortcut_spec = settings_load
        .as_ref()
        .filter(|load| load.warning.is_none())
        .map(|load| ShortcutSpec::from_settings(&load.settings.shortcut))
        .transpose()
        .context("invalid Hyprland shortcut settings")?
        .flatten();

    dispatch_startup(
        startup_action(mode, settings_load.as_ref()),
        move || run_bridge_entrypoint(shortcut_spec),
        run_cleanup,
    )
    .await
}

fn hyprland_ipc() -> anyhow::Result<(HyprlandIpc, bool)> {
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?);
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .context("HYPRLAND_INSTANCE_SIGNATURE is not set")?;
    let paths = SocketPaths::from_values(&runtime, &signature)?;
    let command_socket_exists = paths.command.exists();
    let ipc = HyprlandIpc::new(paths);

    Ok((ipc, command_socket_exists))
}

async fn run_cleanup() -> anyhow::Result<()> {
    let (ipc, command_socket_exists) = hyprland_ipc()?;
    if !command_socket_exists {
        return Ok(());
    }

    cleanup_runtime_state(&ipc).await
}

async fn run_bridge_entrypoint(shortcut_spec: Option<ShortcutSpec>) -> anyhow::Result<()> {
    let log_runtime = match LoggerRuntime::start(Component::Hyprland) {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            eprintln!("OverCrow could not start the Hyprland diagnostic log: {error:#}");
            None
        }
    };
    let logger = log_runtime
        .as_ref()
        .map_or_else(EventLogger::disabled, LoggerRuntime::logger);
    logger.info("process_started", format_args!(""));

    let result = run_bridge_logged(shortcut_spec, &logger).await;
    if let Err(error) = &result {
        logger.error("bridge_failed", format_args!("error={error}"));
    }
    logger.info(
        "process_stopping",
        format_args!("success={}", result.is_ok()),
    );
    result
}

async fn run_bridge_logged(
    shortcut_spec: Option<ShortcutSpec>,
    logger: &EventLogger,
) -> anyhow::Result<()> {
    let (ipc, _) = hyprland_ipc()?;

    let connection = zbus::Connection::session()
        .await
        .context("failed to connect to the user session bus")?;
    let proxy = Core1Proxy::new(&connection)
        .await
        .context("failed to connect to OverCrow Core1")?;

    let result = run_bridge(ipc.clone(), proxy, shortcut_spec, logger.clone()).await;
    if result.is_err()
        && let Err(cleanup_error) = cleanup_runtime_state(&ipc).await
    {
        logger.error("cleanup_failed", format_args!("error={cleanup_error}"));
        eprintln!("OverCrow runtime-state cleanup failed after bridge error: {cleanup_error:#}");
    }
    result
}

fn lifecycle_allows_start(load: &SettingsLoad) -> bool {
    load.warning.is_none() && load.settings.enabled && load.settings.clone().validate().is_ok()
}

fn startup_action(mode: Mode, load: Option<&SettingsLoad>) -> StartupAction {
    match mode {
        Mode::CleanupFocusState => StartupAction::CleanupFocusState,
        Mode::Run if load.is_some_and(lifecycle_allows_start) => StartupAction::Run,
        Mode::Run => StartupAction::Exit,
    }
}

async fn dispatch_startup<E, Run, RunFuture, Cleanup, CleanupFuture>(
    action: StartupAction,
    run: Run,
    cleanup: Cleanup,
) -> Result<(), E>
where
    Run: FnOnce() -> RunFuture,
    RunFuture: std::future::Future<Output = Result<(), E>>,
    Cleanup: FnOnce() -> CleanupFuture,
    CleanupFuture: std::future::Future<Output = Result<(), E>>,
{
    match action {
        StartupAction::Exit => Ok(()),
        StartupAction::Run => run().await,
        StartupAction::CleanupFocusState => cleanup().await,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use overcrow_config::{LifecycleSettings, SettingsLoad};

    use super::{
        Mode, StartupAction, dispatch_startup, lifecycle_allows_start, parse_mode, startup_action,
    };

    fn settings_load(enabled: bool) -> SettingsLoad {
        SettingsLoad {
            settings: LifecycleSettings {
                enabled,
                ..LifecycleSettings::default()
            },
            warning: None,
        }
    }

    #[test]
    fn cleanup_mode_accepts_only_the_explicit_flag() {
        assert_eq!(parse_mode(Vec::<OsString>::new()), Ok(Mode::Run));
        assert_eq!(
            parse_mode([OsString::from("--cleanup-focus-state")]),
            Ok(Mode::CleanupFocusState)
        );
        assert!(parse_mode([OsString::from("--unknown")]).is_err());
        assert!(
            parse_mode([
                OsString::from("--cleanup-focus-state"),
                OsString::from("extra")
            ])
            .is_err()
        );
    }

    #[test]
    fn lifecycle_guard_accepts_only_valid_enabled_settings() {
        assert!(lifecycle_allows_start(&settings_load(true)));
        assert!(!lifecycle_allows_start(&settings_load(false)));

        let mut warned = settings_load(true);
        warned.warning = Some("unsafe settings".to_owned());
        assert!(!lifecycle_allows_start(&warned));

        let mut invalid = settings_load(true);
        invalid.settings.schema_version += 1;
        assert!(!lifecycle_allows_start(&invalid));
    }

    #[tokio::test]
    async fn rejected_settings_do_not_enter_hyprland_startup() {
        let mut run_calls = 0;
        let mut cleanup_calls = 0;
        let action = startup_action(Mode::Run, Some(&settings_load(false)));

        dispatch_startup(
            action,
            || async {
                run_calls += 1;
                Ok::<_, anyhow::Error>(())
            },
            || async {
                cleanup_calls += 1;
                Ok::<_, anyhow::Error>(())
            },
        )
        .await
        .unwrap();

        assert_eq!(action, StartupAction::Exit);
        assert_eq!(run_calls, 0);
        assert_eq!(cleanup_calls, 0);
    }

    #[tokio::test]
    async fn cleanup_dispatch_bypasses_lifecycle_authority() {
        let mode = parse_mode([OsString::from("--cleanup-focus-state")]).unwrap();
        let action = startup_action(mode, None);
        let mut run_calls = 0;
        let mut cleanup_calls = 0;

        dispatch_startup(
            action,
            || async {
                run_calls += 1;
                Ok::<_, anyhow::Error>(())
            },
            || async {
                cleanup_calls += 1;
                Ok::<_, anyhow::Error>(())
            },
        )
        .await
        .unwrap();

        assert_eq!(mode, Mode::CleanupFocusState);
        assert_eq!(action, StartupAction::CleanupFocusState);
        assert_eq!(run_calls, 0);
        assert_eq!(cleanup_calls, 1);
    }
}
