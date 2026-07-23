use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    io,
    os::unix::process::ExitStatusExt,
    pin::Pin,
    process::ExitStatus,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, anyhow};
use overcrow_config::LifecycleSettings;
use overcrow_protocol::{CoreState, OverlayMode, Rect};
use tokio::{
    sync::{Notify, watch},
    task::JoinHandle,
};

use crate::{
    CoreRuntime, ProcessInfo, WindowObservation,
    session::{
        AppliedState, CHILD_REAP_TIMEOUT, CommandRunner, DesktopSession, FORCED_CLEANUP_TIMEOUT,
        MAX_STOP_COMMANDS, ManagedChild, SYSTEMCTL_COMMAND_TIMEOUT, SessionCommand,
        SessionCoordinator, run_owned_child, run_session_coordinator,
        run_session_coordinator_with_retry, shutdown_session_coordinator,
        shutdown_session_coordinator_with_timeouts, transition_commands,
    },
};

const OVERLAY: SessionCommand = SessionCommand::new("start", "overcrow-overlay.service");
const BRIDGE: SessionCommand = SessionCommand::new("start", "overcrow-hyprland.service");
const STOP_OVERLAY: SessionCommand = SessionCommand::new("stop", "overcrow-overlay.service");
const STOP_BRIDGE: SessionCommand = SessionCommand::new("stop", "overcrow-hyprland.service");

#[derive(Default)]
struct FakeRunner {
    calls: Mutex<Vec<SessionCommand>>,
    completed: Mutex<Vec<SessionCommand>>,
    failures: Mutex<VecDeque<bool>>,
    gate: Mutex<Option<Arc<Gate>>>,
    cancelled: AtomicBool,
    passive_observer: Mutex<Option<watch::Receiver<overcrow_protocol::VersionedCoreSnapshot>>>,
    passive_before_stop: Mutex<Vec<bool>>,
}

impl FakeRunner {
    fn failing(sequence: impl IntoIterator<Item = bool>) -> Arc<Self> {
        Arc::new(Self {
            failures: Mutex::new(sequence.into_iter().collect()),
            ..Self::default()
        })
    }

    fn calls(&self) -> Vec<SessionCommand> {
        self.calls.lock().unwrap().clone()
    }

    fn completed(&self) -> Vec<SessionCommand> {
        self.completed.lock().unwrap().clone()
    }
}

struct StubbornFuture {
    dropped: Arc<AtomicBool>,
}

impl Future for StubbornFuture {
    type Output = Result<()>;

    fn poll(
        self: Pin<&mut Self>,
        _context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Pending
    }
}

impl Drop for StubbornFuture {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

struct StubbornOnceRunner {
    calls: Mutex<Vec<SessionCommand>>,
    first: AtomicBool,
    dropped: Arc<AtomicBool>,
}

struct StubbornStartRunner {
    calls: Mutex<Vec<SessionCommand>>,
    first_start: AtomicBool,
    activated: AtomicBool,
    dropped: Arc<AtomicBool>,
}

impl StubbornStartRunner {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            first_start: AtomicBool::new(true),
            activated: AtomicBool::new(false),
            dropped: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl CommandRunner for StubbornStartRunner {
    fn run(
        &self,
        command: SessionCommand,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        self.calls.lock().unwrap().push(command);
        if command == OVERLAY && self.first_start.swap(false, Ordering::SeqCst) {
            self.activated.store(true, Ordering::SeqCst);
            return Box::pin(StubbornFuture {
                dropped: Arc::clone(&self.dropped),
            });
        }
        Box::pin(async { Ok(()) })
    }

    fn cancel(&self) {}

    fn reset_cancellation(&self) {}
}

impl StubbornOnceRunner {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            first: AtomicBool::new(true),
            dropped: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl CommandRunner for StubbornOnceRunner {
    fn run(
        &self,
        command: SessionCommand,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        self.calls.lock().unwrap().push(command);
        if self.first.swap(false, Ordering::SeqCst) {
            return Box::pin(StubbornFuture {
                dropped: Arc::clone(&self.dropped),
            });
        }
        Box::pin(async { Ok(()) })
    }

    fn cancel(&self) {}

    fn reset_cancellation(&self) {}
}

impl CommandRunner for FakeRunner {
    fn run(
        &self,
        command: SessionCommand,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(command);
            if command.verb() == "stop"
                && let Some(snapshot) = self.passive_observer.lock().unwrap().as_ref()
            {
                self.passive_before_stop
                    .lock()
                    .unwrap()
                    .push(snapshot.borrow().snapshot.overlay_mode == OverlayMode::Passive);
            }
            let gate = self.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.wait(&self.cancelled).await;
            }
            if self.cancelled.load(Ordering::SeqCst) {
                return Err(anyhow!("injected cancellation"));
            }
            if self.failures.lock().unwrap().pop_front().unwrap_or(false) {
                return Err(anyhow!("injected command failure"));
            }
            self.completed.lock().unwrap().push(command);
            Ok(())
        })
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.release();
        }
    }

    fn reset_cancellation(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    released: AtomicBool,
    entered_notify: Notify,
    release_notify: Notify,
}

impl Gate {
    async fn wait(&self, cancelled: &AtomicBool) {
        self.entered.store(true, Ordering::SeqCst);
        self.entered_notify.notify_waiters();
        loop {
            let released = self.release_notify.notified();
            if self.released.load(Ordering::SeqCst) || cancelled.load(Ordering::SeqCst) {
                return;
            }
            released.await;
        }
    }

    async fn wait_until_entered(&self) {
        loop {
            let entered = self.entered_notify.notified();
            if self.entered.load(Ordering::SeqCst) {
                return;
            }
            entered.await;
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.release_notify.notify_waiters();
    }
}

#[derive(Default)]
struct FakeChildTrace {
    calls: Mutex<Vec<&'static str>>,
    dropped: AtomicBool,
}

struct FakeChild {
    trace: Arc<FakeChildTrace>,
    waits: VecDeque<io::Result<ExitStatus>>,
    kill: Option<io::Result<()>>,
}

impl Drop for FakeChild {
    fn drop(&mut self) {
        self.trace.dropped.store(true, Ordering::SeqCst);
    }
}

impl ManagedChild for FakeChild {
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + '_>> {
        self.trace.calls.lock().unwrap().push("wait");
        let result = self
            .waits
            .pop_front()
            .unwrap_or_else(|| Ok(ExitStatus::from_raw(0)));
        Box::pin(async move { result })
    }

    fn start_kill(&mut self) -> io::Result<()> {
        self.trace.calls.lock().unwrap().push("kill");
        self.kill.take().unwrap_or(Ok(()))
    }
}

fn fake_child(
    waits: impl IntoIterator<Item = io::Result<ExitStatus>>,
    kill: io::Result<()>,
) -> (FakeChild, Arc<FakeChildTrace>) {
    let trace = Arc::new(FakeChildTrace::default());
    (
        FakeChild {
            trace: Arc::clone(&trace),
            waits: waits.into_iter().collect(),
            kill: Some(kill),
        },
        trace,
    )
}

async fn runtime() -> CoreRuntime {
    CoreRuntime::new(
        Arc::new(tokio::sync::RwLock::new(CoreState::default())),
        HashMap::new(),
    )
    .await
}

async fn interactive_runtime() -> CoreRuntime {
    let mut settings = LifecycleSettings {
        enabled: true,
        ..LifecycleSettings::default()
    };
    settings.selected_steam_app_ids.insert(620);
    let process = ProcessInfo {
        pid: 10,
        parent_pid: 1,
        start_ticks: 0,
        timing: None,
        resources: Default::default(),
        name: "portal2".to_owned(),
        environment: HashMap::from([("SteamAppId".to_owned(), "620".to_owned())]),
        command_line: Vec::new(),
        executable: Some("/games/portal2".into()),
    };
    let runtime = CoreRuntime::with_settings(
        Arc::new(tokio::sync::RwLock::new(CoreState::default())),
        HashMap::from([(10, process)]),
        settings,
    )
    .await;
    runtime
        .apply_x11_observation(Some(WindowObservation {
            pid: Some(10),
            app_id: Some("portal2".to_owned()),
            title: "Portal 2".to_owned(),
            rect: Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            scale: 1.0,
            backend: "x11".to_owned(),
        }))
        .await;
    runtime.toggle_overlay().await;
    assert_eq!(
        runtime.snapshot().await.overlay_mode,
        OverlayMode::Interactive
    );
    runtime
}

#[test]
fn desktop_detection_is_exact_case_insensitive_and_x11_takes_precedence() {
    assert_eq!(
        DesktopSession::detect(Some("X11"), Some("Hyprland"), Some("plasma")),
        DesktopSession::X11
    );
    assert_eq!(
        DesktopSession::detect(Some("wayland"), Some("foo:HYPRLAND"), None),
        DesktopSession::Unsupported
    );
    assert_eq!(
        DesktopSession::detect(Some("WAYLAND"), Some("KDE"), None),
        DesktopSession::PlasmaWayland
    );
    assert_eq!(
        DesktopSession::detect(Some("wayland"), Some("not-hyprland"), Some("plasma-ish")),
        DesktopSession::Unsupported
    );
    assert_eq!(
        DesktopSession::detect(Some("tty"), Some("Hyprland"), None),
        DesktopSession::Unsupported
    );
    assert_eq!(
        DesktopSession::detect(Some("wayland"), Some("hyprland:a:b:c:d:e:f:g:h"), None,),
        DesktopSession::Unsupported
    );
    assert_eq!(
        DesktopSession::detect(Some("wayland"), Some("GNOME"), Some("plasma")),
        DesktopSession::Unsupported
    );
    assert_eq!(
        DesktopSession::detect(Some("wayland"), Some("Hyprland:KDE"), None),
        DesktopSession::Unsupported
    );
    assert_eq!(
        DesktopSession::detect(Some("wayland"), Some(""), Some("plasma")),
        DesktopSession::PlasmaWayland
    );
}

#[test]
fn command_plans_cover_supported_desktops_and_idempotence() {
    assert_eq!(
        transition_commands(DesktopSession::Hyprland, false, true),
        vec![OVERLAY, BRIDGE]
    );
    assert_eq!(
        transition_commands(DesktopSession::Hyprland, true, false),
        vec![STOP_BRIDGE, STOP_OVERLAY]
    );
    for desktop in [DesktopSession::X11, DesktopSession::PlasmaWayland] {
        assert_eq!(transition_commands(desktop, false, true), vec![OVERLAY]);
        assert_eq!(
            transition_commands(desktop, true, false),
            vec![STOP_OVERLAY]
        );
    }
    assert!(transition_commands(DesktopSession::Unsupported, false, true).is_empty());
    assert_eq!(
        transition_commands(DesktopSession::Unsupported, true, false),
        vec![STOP_BRIDGE, STOP_OVERLAY]
    );
    assert!(transition_commands(DesktopSession::Hyprland, false, false).is_empty());
    assert!(transition_commands(DesktopSession::Hyprland, true, true).is_empty());
}

#[test]
fn forced_cleanup_bound_includes_every_stop_timeout_and_scheduling_margin() {
    let command_bound = SYSTEMCTL_COMMAND_TIMEOUT + CHILD_REAP_TIMEOUT;
    for desktop in [
        DesktopSession::X11,
        DesktopSession::Hyprland,
        DesktopSession::PlasmaWayland,
        DesktopSession::Unsupported,
    ] {
        let stop_plan = transition_commands(desktop, true, false);
        assert!(stop_plan.len() <= MAX_STOP_COMMANDS);
        assert!(FORCED_CLEANUP_TIMEOUT > command_bound * stop_plan.len() as u32);
    }
}

#[tokio::test]
async fn child_wait_error_still_attempts_kill_and_bounded_reap() {
    let (child, trace) = fake_child(
        [
            Err(io::Error::other("wait failed")),
            Ok(ExitStatus::from_raw(0)),
        ],
        Ok(()),
    );
    let (_cancel, cancellation) = watch::channel(false);

    let error = run_owned_child(child, "fake child", Duration::from_secs(1), cancellation)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("wait failed"));
    assert_eq!(*trace.calls.lock().unwrap(), vec!["wait", "kill", "wait"]);
    assert!(trace.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn child_kill_error_does_not_skip_reap_and_is_aggregated() {
    let (child, trace) = fake_child(
        [
            Err(io::Error::other("wait failed")),
            Ok(ExitStatus::from_raw(0)),
        ],
        Err(io::Error::other("kill failed")),
    );
    let (_cancel, cancellation) = watch::channel(false);

    let error = run_owned_child(child, "fake child", Duration::from_secs(1), cancellation)
        .await
        .unwrap_err();

    let message = format!("{error:#}");
    assert!(message.contains("wait failed"));
    assert!(message.contains("kill failed"));
    assert_eq!(*trace.calls.lock().unwrap(), vec!["wait", "kill", "wait"]);
    assert!(trace.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn child_reap_error_is_aggregated_after_wait_and_kill_errors() {
    let (child, trace) = fake_child(
        [
            Err(io::Error::other("wait failed")),
            Err(io::Error::other("reap failed")),
        ],
        Err(io::Error::other("kill failed")),
    );
    let (_cancel, cancellation) = watch::channel(false);

    let error = run_owned_child(child, "fake child", Duration::from_secs(1), cancellation)
        .await
        .unwrap_err();

    let message = format!("{error:#}");
    assert!(message.contains("wait failed"));
    assert!(message.contains("kill failed"));
    assert!(message.contains("reap failed"));
    assert_eq!(*trace.calls.lock().unwrap(), vec!["wait", "kill", "wait"]);
    assert!(trace.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn repeated_reconciliation_is_idempotent() {
    let runner = Arc::new(FakeRunner::default());
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );

    coordinator.reconcile(true).await.unwrap();
    coordinator.reconcile(true).await.unwrap();
    coordinator.reconcile(false).await.unwrap();
    coordinator.reconcile(false).await.unwrap();

    assert_eq!(
        runner.calls(),
        vec![
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            BRIDGE,
            STOP_BRIDGE,
            STOP_OVERLAY,
        ]
    );
    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);
}

#[tokio::test]
async fn bootstrap_false_runs_reverse_cleanup_before_claiming_stopped() {
    let runner = Arc::new(FakeRunner::default());
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );

    coordinator.reconcile(false).await.unwrap();

    assert_eq!(runner.calls(), vec![STOP_BRIDGE, STOP_OVERLAY]);
    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);
}

#[tokio::test]
async fn bootstrap_true_cleans_unknown_units_before_starting() {
    let runner = Arc::new(FakeRunner::default());
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );

    coordinator.reconcile(true).await.unwrap();

    assert_eq!(
        runner.calls(),
        vec![STOP_BRIDGE, STOP_OVERLAY, OVERLAY, BRIDGE]
    );
    assert_eq!(coordinator.applied_state(), AppliedState::Running);
}

#[tokio::test]
async fn unsupported_wayland_clears_runtime_state_without_starting_units() {
    let runtime = interactive_runtime().await;
    let runner = Arc::new(FakeRunner::default());
    let coordinator = SessionCoordinator::new(
        runtime.clone(),
        DesktopSession::Unsupported,
        Arc::clone(&runner),
    );

    coordinator.reconcile(true).await.unwrap();

    assert_eq!(runtime.snapshot().await.overlay_mode, OverlayMode::Passive);
    assert!(runtime.snapshot().await.active_game.is_none());
    assert_eq!(runner.calls(), vec![STOP_BRIDGE, STOP_OVERLAY]);

    coordinator.reconcile(true).await.unwrap();
    assert_eq!(runner.calls(), vec![STOP_BRIDGE, STOP_OVERLAY]);

    shutdown_session_coordinator(&coordinator, None, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(
        runner.calls(),
        vec![STOP_BRIDGE, STOP_OVERLAY, STOP_BRIDGE, STOP_OVERLAY]
    );
}

#[tokio::test]
async fn failed_first_start_runs_full_cleanup_and_can_retry() {
    let runner = FakeRunner::failing([false, false, true, false, false, false, false]);
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );

    assert!(coordinator.reconcile(true).await.is_err());
    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);
    coordinator.reconcile(true).await.unwrap();

    assert_eq!(
        runner.calls(),
        vec![
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            BRIDGE,
        ]
    );
    assert_eq!(coordinator.applied_state(), AppliedState::Running);
}

#[tokio::test]
async fn partial_start_runs_full_cleanup_in_reverse_order() {
    let runner = FakeRunner::failing([false, false, false, true, false, false]);
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );

    assert!(coordinator.reconcile(true).await.is_err());

    assert_eq!(
        runner.calls(),
        vec![
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            BRIDGE,
            STOP_BRIDGE,
            STOP_OVERLAY,
        ]
    );
    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);
}

#[tokio::test]
async fn cleanup_failures_remain_retryable_before_a_new_start() {
    let runner = FakeRunner::failing([false, false, true, true, false, false, false, false, false]);
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );

    assert!(coordinator.reconcile(true).await.is_err());
    assert_eq!(coordinator.applied_state(), AppliedState::NeedsCleanup);
    coordinator.reconcile(true).await.unwrap();

    assert_eq!(
        runner.calls(),
        vec![
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            STOP_BRIDGE,
            STOP_OVERLAY,
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            BRIDGE,
        ]
    );
    assert_eq!(coordinator.applied_state(), AppliedState::Running);
}

#[tokio::test]
async fn every_stop_is_attempted_after_individual_failures() {
    let runner = FakeRunner::failing([false, false, false, false, true, true]);
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );
    coordinator.reconcile(true).await.unwrap();

    assert!(coordinator.reconcile(false).await.is_err());

    assert_eq!(
        runner.calls(),
        vec![
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            BRIDGE,
            STOP_BRIDGE,
            STOP_OVERLAY,
        ]
    );
    assert_eq!(coordinator.applied_state(), AppliedState::NeedsCleanup);
}

#[tokio::test]
async fn runtime_is_passive_before_each_stop_command() {
    let runtime = interactive_runtime().await;
    let runner = Arc::new(FakeRunner::default());
    *runner.passive_observer.lock().unwrap() = Some(runtime.snapshots());
    let coordinator =
        SessionCoordinator::new(runtime, DesktopSession::Hyprland, Arc::clone(&runner));
    coordinator.reconcile(true).await.unwrap();

    coordinator.reconcile(false).await.unwrap();

    assert_eq!(
        *runner.passive_before_stop.lock().unwrap(),
        vec![true, true, true, true]
    );
}

#[tokio::test]
async fn initial_true_starts_and_closed_channel_forces_stop() {
    let runner = Arc::new(FakeRunner::default());
    let coordinator =
        SessionCoordinator::new(runtime().await, DesktopSession::X11, Arc::clone(&runner));
    let (sender, receiver) = watch::channel(true);
    drop(sender);

    run_session_coordinator(coordinator, receiver)
        .await
        .unwrap();

    assert_eq!(runner.calls(), vec![STOP_OVERLAY, OVERLAY, STOP_OVERLAY]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rapid_updates_are_coalesced_without_losing_latest_state() {
    let gate = Arc::new(Gate::default());
    let runner = Arc::new(FakeRunner::default());
    *runner.gate.lock().unwrap() = Some(Arc::clone(&gate));
    let coordinator =
        SessionCoordinator::new(runtime().await, DesktopSession::X11, Arc::clone(&runner));
    let (sender, receiver) = watch::channel(true);
    let task = tokio::spawn(run_session_coordinator(coordinator, receiver));
    gate.wait_until_entered().await;

    sender.send(false).unwrap();
    sender.send(true).unwrap();
    gate.release();
    tokio::task::yield_now().await;
    drop(sender);
    task.await.unwrap().unwrap();

    assert_eq!(runner.calls(), vec![STOP_OVERLAY, OVERLAY, STOP_OVERLAY]);
}

#[tokio::test]
async fn unchanged_false_retries_a_transient_cleanup_failure_without_busy_waiting() {
    let runner = FakeRunner::failing([true, false, false, false]);
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );
    let (_sender, receiver) = watch::channel(false);
    let task = tokio::spawn(run_session_coordinator_with_retry(
        coordinator.clone(),
        receiver,
        Duration::ZERO,
    ));

    tokio::time::timeout(Duration::from_secs(1), async {
        while coordinator.applied_state() != AppliedState::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("transient cleanup retry completes");

    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);
    assert_eq!(
        runner.calls(),
        vec![STOP_BRIDGE, STOP_OVERLAY, STOP_BRIDGE, STOP_OVERLAY]
    );
    coordinator.request_shutdown();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn unchanged_true_retries_a_transient_start_failure() {
    let runner = FakeRunner::failing([false, false, true, false, false, false, false]);
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );
    let (_sender, receiver) = watch::channel(true);
    let task = tokio::spawn(run_session_coordinator_with_retry(
        coordinator.clone(),
        receiver,
        Duration::ZERO,
    ));

    tokio::time::timeout(Duration::from_secs(1), async {
        while coordinator.applied_state() != AppliedState::Running {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("transient start retry completes");

    assert_eq!(coordinator.applied_state(), AppliedState::Running);
    assert_eq!(
        runner.calls(),
        vec![
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            BRIDGE,
        ]
    );
    coordinator.request_shutdown();
    task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_timeout_cancels_a_blocked_runner_and_joins_the_task() {
    let gate = Arc::new(Gate::default());
    let runner = Arc::new(FakeRunner::default());
    *runner.gate.lock().unwrap() = Some(Arc::clone(&gate));
    let coordinator =
        SessionCoordinator::new(runtime().await, DesktopSession::X11, Arc::clone(&runner));
    let (_sender, receiver) = watch::channel(true);
    let mut task: JoinHandle<Result<()>> =
        tokio::spawn(run_session_coordinator(coordinator.clone(), receiver));
    gate.wait_until_entered().await;

    let result = shutdown_session_coordinator(&coordinator, Some(&mut task), Duration::ZERO).await;

    assert!(result.is_err());
    assert!(!runner.cancelled.load(Ordering::SeqCst));
    assert!(task.is_finished());
}

#[tokio::test]
async fn cancel_grace_expiry_drops_owned_future_then_still_forces_reverse_cleanup() {
    let runner = StubbornOnceRunner::new();
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );
    let (_sender, receiver) = watch::channel(false);
    let mut task = tokio::spawn(run_session_coordinator(coordinator.clone(), receiver));
    while runner.calls.lock().unwrap().is_empty() {
        tokio::task::yield_now().await;
    }

    let result = shutdown_session_coordinator_with_timeouts(
        &coordinator,
        Some(&mut task),
        Duration::ZERO,
        Duration::ZERO,
        Duration::from_secs(1),
    )
    .await;

    assert!(result.is_err());
    assert!(runner.dropped.load(Ordering::SeqCst));
    assert!(task.is_finished());
    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);
    assert_eq!(
        *runner.calls.lock().unwrap(),
        vec![STOP_BRIDGE, STOP_BRIDGE, STOP_OVERLAY]
    );
}

#[tokio::test]
async fn pending_external_start_is_unconditionally_reverse_stopped_after_abort() {
    let runner = StubbornStartRunner::new();
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );
    let (_sender, receiver) = watch::channel(true);
    let mut task = tokio::spawn(run_session_coordinator(coordinator.clone(), receiver));
    while !runner.activated.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);

    let result = shutdown_session_coordinator_with_timeouts(
        &coordinator,
        Some(&mut task),
        Duration::ZERO,
        Duration::ZERO,
        Duration::from_secs(1),
    )
    .await;

    assert!(result.is_err());
    assert!(runner.dropped.load(Ordering::SeqCst));
    assert!(task.is_finished());
    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);
    assert_eq!(
        *runner.calls.lock().unwrap(),
        vec![
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            STOP_BRIDGE,
            STOP_OVERLAY,
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_shutdown_rearms_the_runner_and_performs_real_reverse_cleanup() {
    let gate = Arc::new(Gate::default());
    let runner = Arc::new(FakeRunner::default());
    *runner.gate.lock().unwrap() = Some(Arc::clone(&gate));
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );
    let (_sender, receiver) = watch::channel(true);
    let mut task = tokio::spawn(run_session_coordinator(coordinator.clone(), receiver));
    gate.wait_until_entered().await;

    let result = shutdown_session_coordinator(&coordinator, Some(&mut task), Duration::ZERO).await;

    assert!(result.is_err());
    assert_eq!(runner.completed(), vec![STOP_BRIDGE, STOP_OVERLAY]);
    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);
    assert!(task.is_finished());
}

#[tokio::test]
async fn shutdown_without_a_task_handle_still_performs_bounded_cleanup() {
    let runner = Arc::new(FakeRunner::default());
    let coordinator = SessionCoordinator::new(
        runtime().await,
        DesktopSession::Hyprland,
        Arc::clone(&runner),
    );
    coordinator.reconcile(true).await.unwrap();

    shutdown_session_coordinator(&coordinator, None, Duration::from_secs(1))
        .await
        .unwrap();

    assert_eq!(coordinator.applied_state(), AppliedState::Stopped);
    assert_eq!(
        runner.calls(),
        vec![
            STOP_BRIDGE,
            STOP_OVERLAY,
            OVERLAY,
            BRIDGE,
            STOP_BRIDGE,
            STOP_OVERLAY,
        ]
    );
}
