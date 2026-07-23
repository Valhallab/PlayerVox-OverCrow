use std::{collections::HashMap, future::Future, sync::Arc};

use anyhow::Context;
use overcrow_config::{
    LifecycleSettings, SettingsLoad, SettingsStore, WidgetProfile, WidgetSettingsStore,
};
use overcrow_core::{
    CoreRuntime, CoreService, DbusSnapshotSignalSink, DesktopSession, PortalShortcutBroker,
    ProcessInfo, SESSION_SHUTDOWN_TIMEOUT, SHORTCUT_SHUTDOWN_TIMEOUT, SessionCoordinator,
    ShortcutError, SystemctlRunner, X11WindowSource, run_bridge_watchdog, run_core_event_logging,
    run_process_refresh, run_session_coordinator, run_snapshot_signal_publisher,
    run_widget_settings_refresh, run_window_polling, scan_processes, should_use_x11_source,
    shutdown_session_coordinator,
};
use overcrow_logging::{Component, EventLogger, LoggerRuntime};
use overcrow_protocol::{Core1Proxy, CoreState};
use tokio::sync::{RwLock, watch};
use zbus::proxy::Defaults;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let settings_store = Arc::new(SettingsStore::from_environment());
    let settings_load = settings_store.load();
    if let Some(warning) = &settings_load.warning {
        eprintln!("OverCrow lifecycle settings rejected; remaining inert: {warning}");
    }
    let widget_settings_store = Arc::new(WidgetSettingsStore::from_environment());
    let widget_settings_load = widget_settings_store.load();
    if let Some(warning) = &widget_settings_load.warning {
        eprintln!("OverCrow widget settings rejected; using defaults: {warning}");
    }

    run_if_lifecycle_allows(settings_load, |settings| {
        run_core(
            settings_store,
            widget_settings_store,
            settings,
            widget_settings_load.profile,
        )
    })
    .await
}

async fn run_core(
    settings_store: Arc<SettingsStore>,
    widget_settings_store: Arc<WidgetSettingsStore>,
    settings: LifecycleSettings,
    widget_profile: WidgetProfile,
) -> anyhow::Result<()> {
    let log_runtime = match LoggerRuntime::start(Component::Core) {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            eprintln!("OverCrow diagnostic logger failed to start: {error}");
            None
        }
    };
    let logger = log_runtime
        .as_ref()
        .map(LoggerRuntime::logger)
        .unwrap_or_else(EventLogger::disabled);
    let desktop_session = DesktopSession::from_environment();
    logger.info(
        "process_started",
        format_args!("desktop_session={desktop_session:?}"),
    );
    let state = Arc::new(RwLock::new(CoreState::default()));
    let processes = initial_processes_with(scan_processes, |message| eprintln!("{message}"));
    let runtime =
        CoreRuntime::with_settings_and_widget_profile(state, processes, settings, widget_profile)
            .await;
    let snapshot_receiver = runtime.snapshots();
    let event_logging = run_core_event_logging(runtime.snapshots(), logger.clone());
    tokio::pin!(event_logging);
    let service = CoreService::with_runtime_and_stores(
        runtime.clone(),
        settings_store,
        widget_settings_store,
    );
    let destination = Core1Proxy::DESTINATION
        .as_ref()
        .context("Core1 proxy is missing its default D-Bus destination")?
        .clone();
    let path = Core1Proxy::PATH
        .as_ref()
        .context("Core1 proxy is missing its default D-Bus path")?
        .clone();

    let connection = zbus::connection::Builder::session()?
        .name(destination)?
        .serve_at(path.clone(), service.clone())?
        .build()
        .await?;
    let core_interface = connection
        .object_server()
        .interface::<_, CoreService>(&path)
        .await
        .context("failed to obtain the served Core1 interface")?;
    let snapshot_signal_sink = Arc::new(DbusSnapshotSignalSink::new(
        core_interface.signal_emitter().clone(),
    ));
    let snapshot_publisher = run_snapshot_signal_publisher(snapshot_receiver, snapshot_signal_sink);

    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let use_x11 = should_use_x11_source(session_type.as_deref());
    let polling_runtime = runtime.clone();
    let polling = async move {
        if use_x11 {
            match X11WindowSource::connect() {
                Ok(source) => run_window_polling(source, polling_runtime).await,
                Err(error) => {
                    eprintln!("OverCrow X11 source unavailable; remaining passive: {error:#}");
                    std::future::pending::<()>().await;
                }
            }
        } else {
            std::future::pending::<()>().await;
        }
    };
    let watchdog_runtime = runtime.clone();
    let watchdog = async move {
        if use_x11 {
            std::future::pending::<()>().await;
        } else {
            run_bridge_watchdog(watchdog_runtime).await;
        }
    };
    let process_refresh = run_process_refresh(runtime.clone());
    let widget_settings_refresh = run_widget_settings_refresh(service);
    let selected_processes = runtime.selected_processes_running();
    let (shortcut_shutdown_tx, shortcut_shutdown_rx) = watch::channel(false);
    let shortcut_broker = PortalShortcutBroker::new(runtime.clone());
    let mut shortcut_task = tokio::spawn(shortcut_broker.run(shortcut_shutdown_rx));
    let coordinator = SessionCoordinator::new(
        runtime,
        desktop_session,
        Arc::new(SystemctlRunner::default()),
    );
    let mut session_task = tokio::spawn(run_session_coordinator(
        coordinator.clone(),
        selected_processes,
    ));

    let termination = tokio::select! {
        () = polling => CoreTermination::PollingEnded,
        () = watchdog => CoreTermination::WatchdogEnded,
        () = process_refresh => CoreTermination::ProcessRefreshEnded,
        () = widget_settings_refresh => CoreTermination::WidgetSettingsRefreshEnded,
        () = snapshot_publisher => CoreTermination::SnapshotPublisherEnded,
        () = &mut event_logging => CoreTermination::EventLoggingEnded,
        signal = shutdown_signal() => CoreTermination::Signal(signal),
        result = &mut session_task => CoreTermination::Session(result),
        result = &mut shortcut_task => CoreTermination::Shortcut(result),
    };

    let session_consumed = termination.session_task_consumed();
    let shortcut_consumed = termination.shortcut_task_consumed();
    let termination_name = match &termination {
        CoreTermination::PollingEnded => "polling_ended",
        CoreTermination::WatchdogEnded => "watchdog_ended",
        CoreTermination::ProcessRefreshEnded => "process_refresh_ended",
        CoreTermination::WidgetSettingsRefreshEnded => "widget_settings_refresh_ended",
        CoreTermination::SnapshotPublisherEnded => "snapshot_publisher_ended",
        CoreTermination::EventLoggingEnded => "event_logging_ended",
        CoreTermination::Signal(_) => "signal",
        CoreTermination::Session(_) => "session_coordinator_ended",
        CoreTermination::Shortcut(_) => "shortcut_broker_ended",
    };
    logger.info(
        "process_stopping",
        format_args!("reason={termination_name}"),
    );
    shortcut_shutdown_tx.send_replace(true);
    let session_cleanup = shutdown_session_coordinator(
        &coordinator,
        if session_consumed {
            None
        } else {
            Some(&mut session_task)
        },
        SESSION_SHUTDOWN_TIMEOUT,
    );
    let shortcut_cleanup = shutdown_shortcut_broker(
        if shortcut_consumed {
            None
        } else {
            Some(&mut shortcut_task)
        },
        SHORTCUT_SHUTDOWN_TIMEOUT,
    );
    finalize_termination(termination, async {
        let (session, shortcut) = tokio::join!(session_cleanup, shortcut_cleanup);
        aggregate_cleanup_results(session, shortcut)
    })
    .await
}

async fn shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .context("failed to listen for SIGTERM")?;
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.context("failed to listen for Ctrl-C")
            }
            signal = terminate.recv() => {
                signal.context("SIGTERM listener closed unexpectedly")
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to listen for Ctrl-C")
    }
}

fn lifecycle_allows_start(load: &SettingsLoad) -> bool {
    load.warning.is_none() && load.settings.enabled && load.settings.clone().validate().is_ok()
}

async fn run_if_lifecycle_allows<E, F, Fut>(load: SettingsLoad, start: F) -> Result<(), E>
where
    F: FnOnce(LifecycleSettings) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    if !lifecycle_allows_start(&load) {
        return Ok(());
    }

    start(load.settings).await
}

enum CoreTermination {
    PollingEnded,
    WatchdogEnded,
    ProcessRefreshEnded,
    WidgetSettingsRefreshEnded,
    SnapshotPublisherEnded,
    EventLoggingEnded,
    Signal(anyhow::Result<()>),
    Session(Result<anyhow::Result<()>, tokio::task::JoinError>),
    Shortcut(Result<Result<(), ShortcutError>, tokio::task::JoinError>),
}

impl CoreTermination {
    fn session_task_consumed(&self) -> bool {
        matches!(self, Self::Session(_))
    }

    fn shortcut_task_consumed(&self) -> bool {
        matches!(self, Self::Shortcut(_))
    }

    fn into_primary_result(self) -> anyhow::Result<()> {
        match self {
            Self::PollingEnded => anyhow::bail!("window polling stopped unexpectedly"),
            Self::WatchdogEnded => anyhow::bail!("bridge watchdog stopped unexpectedly"),
            Self::ProcessRefreshEnded => anyhow::bail!("process refresh stopped unexpectedly"),
            Self::WidgetSettingsRefreshEnded => {
                anyhow::bail!("widget settings refresh stopped unexpectedly")
            }
            Self::SnapshotPublisherEnded => {
                anyhow::bail!("snapshot signal publisher stopped unexpectedly")
            }
            Self::EventLoggingEnded => {
                anyhow::bail!("diagnostic event logging stopped unexpectedly")
            }
            Self::Signal(result) => result,
            Self::Session(Ok(Ok(()))) => {
                anyhow::bail!("session coordinator stopped unexpectedly")
            }
            Self::Session(Ok(Err(error))) => Err(error).context("session coordinator failed"),
            Self::Session(Err(error)) => Err(error).context("session coordinator task failed"),
            Self::Shortcut(Ok(Ok(()))) => {
                anyhow::bail!("shortcut broker stopped unexpectedly")
            }
            Self::Shortcut(Ok(Err(error))) => Err(error).context("shortcut broker failed"),
            Self::Shortcut(Err(error)) => Err(error).context("shortcut broker task failed"),
        }
    }
}

async fn shutdown_shortcut_broker(
    task: Option<&mut tokio::task::JoinHandle<Result<(), ShortcutError>>>,
    timeout: std::time::Duration,
) -> anyhow::Result<()> {
    let Some(task) = task else {
        return Ok(());
    };
    match tokio::time::timeout(timeout, &mut *task).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(error))) => Err(error).context("shortcut broker cleanup failed"),
        Ok(Err(error)) => Err(error).context("shortcut broker cleanup task failed"),
        Err(_) => {
            task.abort();
            let _ = task.await;
            anyhow::bail!("shortcut broker cleanup timed out")
        }
    }
}

fn aggregate_cleanup_results(
    session: anyhow::Result<()>,
    shortcut: anyhow::Result<()>,
) -> anyhow::Result<()> {
    match (session, shortcut) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error).context("session cleanup failed"),
        (Ok(()), Err(error)) => Err(error).context("shortcut cleanup failed"),
        (Err(session), Err(shortcut)) => Err(anyhow::anyhow!(
            "session cleanup failed: {session:#}; shortcut cleanup failed: {shortcut:#}"
        )),
    }
}

async fn finalize_termination(
    termination: CoreTermination,
    cleanup: impl Future<Output = anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let primary = termination.into_primary_result();
    let cleanup = cleanup.await;
    match (primary, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(cleanup)) => Err(cleanup).context("failed to stop OverCrow runtime services"),
        (Err(primary), Err(cleanup)) => Err(anyhow::anyhow!(
            "{primary:#}; runtime cleanup also failed: {cleanup:#}"
        )),
    }
}

fn initial_processes_with(
    scan: impl FnOnce() -> anyhow::Result<HashMap<u32, ProcessInfo>>,
    log: impl FnOnce(&str),
) -> HashMap<u32, ProcessInfo> {
    match scan() {
        Ok(processes) => processes,
        Err(error) => {
            log(&format!(
                "OverCrow initial procfs scan failed; remaining passive with an empty process cache: {error:#}"
            ));
            HashMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use anyhow::anyhow;
    use overcrow_config::{LifecycleSettings, SettingsLoad};
    use overcrow_core::ShortcutError;

    use super::{
        CoreTermination, aggregate_cleanup_results, finalize_termination, initial_processes_with,
        lifecycle_allows_start, run_if_lifecycle_allows, shutdown_shortcut_broker,
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

    async fn finalize_with_count(
        termination: CoreTermination,
        cleanup: anyhow::Result<()>,
        calls: Arc<AtomicUsize>,
    ) -> anyhow::Result<()> {
        finalize_termination(termination, async move {
            calls.fetch_add(1, Ordering::SeqCst);
            cleanup
        })
        .await
    }

    #[tokio::test]
    async fn every_termination_branch_runs_cleanup_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let panic = tokio::spawn(async { panic!("coordinator panic") })
            .await
            .unwrap_err();
        let shortcut_panic = tokio::spawn(async { panic!("shortcut panic") })
            .await
            .unwrap_err();
        let terminations = [
            CoreTermination::PollingEnded,
            CoreTermination::WatchdogEnded,
            CoreTermination::ProcessRefreshEnded,
            CoreTermination::WidgetSettingsRefreshEnded,
            CoreTermination::SnapshotPublisherEnded,
            CoreTermination::EventLoggingEnded,
            CoreTermination::Signal(Ok(())),
            CoreTermination::Signal(Err(anyhow!("signal listener failed"))),
            CoreTermination::Session(Ok(Ok(()))),
            CoreTermination::Session(Ok(Err(anyhow!("coordinator failed")))),
            CoreTermination::Session(Err(panic)),
            CoreTermination::Shortcut(Ok(Ok(()))),
            CoreTermination::Shortcut(Ok(Err(ShortcutError::new("portal failed")))),
            CoreTermination::Shortcut(Err(shortcut_panic)),
        ];

        for termination in terminations {
            let _ = finalize_with_count(termination, Ok(()), Arc::clone(&calls)).await;
        }

        assert_eq!(calls.load(Ordering::SeqCst), 14);
    }

    #[test]
    fn snapshot_publisher_end_is_a_runtime_failure() {
        let error = CoreTermination::SnapshotPublisherEnded
            .into_primary_result()
            .unwrap_err();

        assert!(format!("{error:#}").contains("snapshot signal publisher stopped unexpectedly"));
    }

    #[tokio::test]
    async fn primary_and_cleanup_errors_are_both_preserved() {
        let error = finalize_termination(
            CoreTermination::Signal(Err(anyhow!("primary signal error"))),
            async { Err(anyhow!("cleanup error")) },
        )
        .await
        .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("primary signal error"));
        assert!(message.contains("cleanup error"));
    }

    #[tokio::test]
    async fn successful_ctrl_c_returns_a_cleanup_failure() {
        let error = finalize_termination(CoreTermination::Signal(Ok(())), async {
            Err(anyhow!("cleanup failed"))
        })
        .await
        .unwrap_err();

        assert!(format!("{error:#}").contains("cleanup failed"));
    }

    #[test]
    fn only_the_session_branch_consumes_its_join_handle() {
        assert!(!CoreTermination::PollingEnded.session_task_consumed());
        assert!(!CoreTermination::Signal(Ok(())).session_task_consumed());
        assert!(CoreTermination::Session(Ok(Ok(()))).session_task_consumed());
        assert!(!CoreTermination::Shortcut(Ok(Ok(()))).session_task_consumed());
        assert!(CoreTermination::Shortcut(Ok(Ok(()))).shortcut_task_consumed());
    }

    #[test]
    fn session_and_shortcut_cleanup_errors_are_aggregated() {
        let error = aggregate_cleanup_results(
            Err(anyhow!("session cleanup failed")),
            Err(anyhow!("shortcut cleanup failed")),
        )
        .unwrap_err();
        let message = format!("{error:#}");

        assert!(message.contains("session cleanup failed"));
        assert!(message.contains("shortcut cleanup failed"));
    }

    #[tokio::test]
    async fn shortcut_cleanup_is_bounded_and_preserves_broker_errors() {
        let mut failed =
            tokio::spawn(async { Err::<(), _>(ShortcutError::new("portal close failed")) });
        let error = shutdown_shortcut_broker(Some(&mut failed), std::time::Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("portal close failed"));

        let mut stuck =
            tokio::spawn(async { std::future::pending::<Result<(), ShortcutError>>().await });
        let error =
            shutdown_shortcut_broker(Some(&mut stuck), std::time::Duration::from_millis(20))
                .await
                .unwrap_err();
        assert!(format!("{error:#}").contains("timed out"));
        assert!(stuck.is_finished());
    }

    #[test]
    fn failed_initial_scan_logs_and_starts_with_an_empty_cache() {
        let mut messages = Vec::new();

        let processes = initial_processes_with(
            || Err(anyhow!("procfs unavailable")),
            |message| messages.push(message.to_owned()),
        );

        assert!(processes.is_empty());
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("OverCrow initial procfs scan failed"));
        assert!(messages[0].contains("remaining passive"));
        assert!(messages[0].contains("procfs unavailable"));
    }

    #[test]
    fn lifecycle_guard_fails_closed() {
        let disabled = settings_load(false);
        assert!(!lifecycle_allows_start(&disabled));

        let mut warned = settings_load(true);
        warned.warning = Some("unsafe settings".to_owned());
        assert!(!lifecycle_allows_start(&warned));

        let mut invalid = settings_load(true);
        invalid.settings.schema_version += 1;
        assert!(!lifecycle_allows_start(&invalid));
    }

    #[test]
    fn lifecycle_guard_accepts_valid_enabled_settings() {
        assert!(lifecycle_allows_start(&settings_load(true)));
    }

    #[tokio::test]
    async fn rejected_settings_do_not_enter_core_startup() {
        let mut entered = false;

        run_if_lifecycle_allows(settings_load(false), |_| async {
            entered = true;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .unwrap();

        assert!(!entered);
    }
}
