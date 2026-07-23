use std::{
    fmt,
    future::Future,
    io,
    pin::Pin,
    process::{ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use tokio::{process::Command, sync::watch, task::JoinHandle};

use crate::CoreRuntime;

const SYSTEMCTL_PROGRAM: &str = "systemctl";
const OVERLAY_UNIT: &str = "overcrow-overlay.service";
const HYPRLAND_UNIT: &str = "overcrow-hyprland.service";
const SYSTEMCTL_COMMAND_TIMEOUT_SECONDS: u64 = 3;
const CHILD_REAP_TIMEOUT_SECONDS: u64 = 1;
pub(crate) const SYSTEMCTL_COMMAND_TIMEOUT: Duration =
    Duration::from_secs(SYSTEMCTL_COMMAND_TIMEOUT_SECONDS);
pub(crate) const CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(CHILD_REAP_TIMEOUT_SECONDS);
const SHUTDOWN_CANCEL_GRACE: Duration = Duration::from_secs(1);
const FORCED_CLEANUP_MARGIN_SECONDS: u64 = 2;
const RECONCILIATION_RETRY_DELAY: Duration = Duration::from_millis(250);
pub const SESSION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_DESKTOP_METADATA_BYTES: usize = 128;
const MAX_DESKTOP_TOKENS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopSession {
    X11,
    Hyprland,
    PlasmaWayland,
    Unsupported,
}

impl DesktopSession {
    pub fn from_environment() -> Self {
        Self::detect(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
            std::env::var("DESKTOP_SESSION").ok().as_deref(),
        )
    }

    pub fn detect(
        session_type: Option<&str>,
        current_desktop: Option<&str>,
        desktop_session: Option<&str>,
    ) -> Self {
        let Some(session_type) = bounded_token(session_type, 32) else {
            return Self::Unsupported;
        };
        if session_type.eq_ignore_ascii_case("x11") {
            return Self::X11;
        }
        if !session_type.eq_ignore_ascii_case("wayland") {
            return Self::Unsupported;
        }

        let desktop = match parse_desktop_metadata(current_desktop, true) {
            DesktopMetadata::Known(kind) => Some(kind),
            DesktopMetadata::Absent => match parse_desktop_metadata(desktop_session, false) {
                DesktopMetadata::Known(kind) => Some(kind),
                DesktopMetadata::Absent | DesktopMetadata::Invalid => None,
            },
            DesktopMetadata::Invalid => None,
        };
        match desktop {
            Some(DesktopKind::Hyprland) => Self::Hyprland,
            Some(DesktopKind::Plasma) => Self::PlasmaWayland,
            None => Self::Unsupported,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DesktopKind {
    Hyprland,
    Plasma,
}

#[derive(Clone, Copy)]
enum DesktopMetadata {
    Absent,
    Known(DesktopKind),
    Invalid,
}

fn bounded_token(value: Option<&str>, max_bytes: usize) -> Option<&str> {
    let value = value?.trim();
    (!value.is_empty() && value.len() <= max_bytes).then_some(value)
}

fn parse_desktop_metadata(value: Option<&str>, split_tokens: bool) -> DesktopMetadata {
    let Some(raw) = value else {
        return DesktopMetadata::Absent;
    };
    let value = raw.trim();
    if value.is_empty() {
        return DesktopMetadata::Absent;
    }
    if value.len() > MAX_DESKTOP_METADATA_BYTES {
        return DesktopMetadata::Invalid;
    }
    if !split_tokens {
        return desktop_kind(value)
            .map(DesktopMetadata::Known)
            .unwrap_or(DesktopMetadata::Invalid);
    }
    let tokens = value
        .split(':')
        .take(MAX_DESKTOP_TOKENS + 1)
        .collect::<Vec<_>>();
    if tokens.len() > MAX_DESKTOP_TOKENS {
        return DesktopMetadata::Invalid;
    }

    let mut detected = None;
    for token in tokens {
        let Some(kind) = desktop_kind(token.trim()) else {
            return DesktopMetadata::Invalid;
        };
        if detected.is_some_and(|current| current != kind) {
            return DesktopMetadata::Invalid;
        }
        detected = Some(kind);
    }
    detected
        .map(DesktopMetadata::Known)
        .unwrap_or(DesktopMetadata::Invalid)
}

fn desktop_kind(token: &str) -> Option<DesktopKind> {
    if token.eq_ignore_ascii_case("hyprland") {
        return Some(DesktopKind::Hyprland);
    }
    [
        "kde",
        "plasma",
        "plasmawayland",
        "plasma-wayland",
        "plasma6",
        "kde-plasma",
        "kde-plasma-wayland",
    ]
    .iter()
    .any(|known| token.eq_ignore_ascii_case(known))
    .then_some(DesktopKind::Plasma)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionCommand {
    verb: &'static str,
    unit: &'static str,
}

impl SessionCommand {
    pub(crate) const fn new(verb: &'static str, unit: &'static str) -> Self {
        Self { verb, unit }
    }

    pub const fn verb(self) -> &'static str {
        self.verb
    }

    pub const fn unit(self) -> &'static str {
        self.unit
    }
}

impl fmt::Display for SessionCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "systemctl --user {} {}", self.verb, self.unit)
    }
}

const NO_COMMANDS: [SessionCommand; 0] = [];
const OVERLAY_START_PLAN: [SessionCommand; 1] = [SessionCommand::new("start", OVERLAY_UNIT)];
const HYPRLAND_START_PLAN: [SessionCommand; 2] = [
    SessionCommand::new("start", OVERLAY_UNIT),
    SessionCommand::new("start", HYPRLAND_UNIT),
];
const OVERLAY_STOP_PLAN: [SessionCommand; 1] = [SessionCommand::new("stop", OVERLAY_UNIT)];
const FULL_STOP_PLAN: [SessionCommand; 2] = [
    SessionCommand::new("stop", HYPRLAND_UNIT),
    SessionCommand::new("stop", OVERLAY_UNIT),
];

const STOP_PLAN_LENGTHS: [usize; 4] = [
    OVERLAY_STOP_PLAN.len(),
    FULL_STOP_PLAN.len(),
    OVERLAY_STOP_PLAN.len(),
    FULL_STOP_PLAN.len(),
];

const fn maximum_plan_length(lengths: &[usize]) -> usize {
    let mut maximum = 0;
    let mut index = 0;
    while index < lengths.len() {
        if lengths[index] > maximum {
            maximum = lengths[index];
        }
        index += 1;
    }
    maximum
}

pub(crate) const MAX_STOP_COMMANDS: usize = maximum_plan_length(&STOP_PLAN_LENGTHS);
pub(crate) const FORCED_CLEANUP_TIMEOUT: Duration = Duration::from_secs(
    (SYSTEMCTL_COMMAND_TIMEOUT_SECONDS + CHILD_REAP_TIMEOUT_SECONDS) * MAX_STOP_COMMANDS as u64
        + FORCED_CLEANUP_MARGIN_SECONDS,
);

pub fn transition_commands(
    desktop: DesktopSession,
    applied_running: bool,
    desired_running: bool,
) -> Vec<SessionCommand> {
    if applied_running == desired_running {
        return Vec::new();
    }
    if desired_running {
        start_command_plan(desktop).to_vec()
    } else {
        stop_command_plan(desktop).to_vec()
    }
}

fn start_command_plan(desktop: DesktopSession) -> &'static [SessionCommand] {
    match desktop {
        DesktopSession::Hyprland => &HYPRLAND_START_PLAN,
        DesktopSession::X11 | DesktopSession::PlasmaWayland => &OVERLAY_START_PLAN,
        DesktopSession::Unsupported => &NO_COMMANDS,
    }
}

fn stop_command_plan(desktop: DesktopSession) -> &'static [SessionCommand] {
    match desktop {
        DesktopSession::Hyprland | DesktopSession::Unsupported => &FULL_STOP_PLAN,
        DesktopSession::X11 | DesktopSession::PlasmaWayland => &OVERLAY_STOP_PLAN,
    }
}

pub type CommandFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
pub(crate) type ChildWaitFuture<'a> =
    Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + 'a>>;

pub trait CommandRunner: Send + Sync + 'static {
    fn run(&self, command: SessionCommand) -> CommandFuture<'_>;

    fn cancel(&self);

    fn reset_cancellation(&self);
}

pub struct SystemctlRunner {
    timeout: Duration,
    cancelled: AtomicBool,
    cancellation_tx: watch::Sender<bool>,
}

impl Default for SystemctlRunner {
    fn default() -> Self {
        let (cancellation_tx, _) = watch::channel(false);
        Self {
            timeout: SYSTEMCTL_COMMAND_TIMEOUT,
            cancelled: AtomicBool::new(false),
            cancellation_tx,
        }
    }
}

impl CommandRunner for SystemctlRunner {
    fn run(&self, command: SessionCommand) -> CommandFuture<'_> {
        Box::pin(async move {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(anyhow!("systemctl command cancelled before launch"));
            }

            let mut process = Command::new(SYSTEMCTL_PROGRAM);
            process
                .args(["--user", command.verb(), command.unit()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let child = process
                .spawn()
                .with_context(|| format!("failed to launch {command}"))?;
            run_owned_child(
                TokioManagedChild(child),
                &command.to_string(),
                self.timeout,
                self.cancellation_tx.subscribe(),
            )
            .await
        })
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancellation_tx.send_replace(true);
    }

    fn reset_cancellation(&self) {
        self.cancelled.store(false, Ordering::Release);
        self.cancellation_tx.send_replace(false);
    }
}

pub(crate) trait ManagedChild: Send {
    fn wait(&mut self) -> ChildWaitFuture<'_>;
    fn start_kill(&mut self) -> io::Result<()>;
}

struct TokioManagedChild(tokio::process::Child);

impl ManagedChild for TokioManagedChild {
    fn wait(&mut self) -> ChildWaitFuture<'_> {
        Box::pin(self.0.wait())
    }

    fn start_kill(&mut self) -> io::Result<()> {
        self.0.start_kill()
    }
}

pub(crate) async fn run_owned_child<C: ManagedChild>(
    mut child: C,
    label: &str,
    timeout: Duration,
    mut cancellation: watch::Receiver<bool>,
) -> Result<()> {
    enum Completion {
        Wait(io::Result<ExitStatus>),
        Cancelled,
        TimedOut,
    }

    let completion = if *cancellation.borrow_and_update() {
        Completion::Cancelled
    } else {
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            let completion = tokio::select! {
                biased;
                changed = cancellation.changed() => {
                    if changed.is_err() || *cancellation.borrow_and_update() {
                        Some(Completion::Cancelled)
                    } else {
                        None
                    }
                }
                result = child.wait() => Some(Completion::Wait(result)),
                () = &mut deadline => Some(Completion::TimedOut),
            };
            if let Some(completion) = completion {
                break completion;
            }
        }
    };

    match completion {
        Completion::Wait(Ok(status)) => {
            anyhow::ensure!(status.success(), "{label} exited with {status}");
            Ok(())
        }
        Completion::Wait(Err(error)) => {
            let primary = anyhow!("failed to wait for {label}: {error}");
            with_child_cleanup(primary, cleanup_child(&mut child, label).await)
        }
        Completion::Cancelled => {
            let primary = anyhow!("{label} cancelled");
            with_child_cleanup(primary, cleanup_child(&mut child, label).await)
        }
        Completion::TimedOut => {
            let primary = anyhow!("{label} timed out");
            with_child_cleanup(primary, cleanup_child(&mut child, label).await)
        }
    }
}

async fn cleanup_child<C: ManagedChild>(child: &mut C, label: &str) -> Result<()> {
    let kill_error = child.start_kill().err();
    let reap_error = match tokio::time::timeout(CHILD_REAP_TIMEOUT, child.wait()).await {
        Ok(Ok(_)) => None,
        Ok(Err(error)) => Some(anyhow!("failed to reap {label}: {error}")),
        Err(_) => Some(anyhow!("timed out while reaping {label}")),
    };

    match (kill_error, reap_error) {
        (None, None) => Ok(()),
        (Some(kill), None) => Err(anyhow!("failed to kill {label}: {kill}")),
        (None, Some(reap)) => Err(reap),
        (Some(kill), Some(reap)) => Err(anyhow!(
            "failed to kill {label}: {kill}; child reap also failed: {reap:#}"
        )),
    }
}

fn with_child_cleanup(primary: anyhow::Error, cleanup: Result<()>) -> Result<()> {
    match cleanup {
        Ok(()) => Err(primary),
        Err(cleanup) => Err(anyhow!(
            "{primary:#}; child cleanup also failed: {cleanup:#}"
        )),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AppliedState {
    Stopped = 0,
    Running = 1,
    NeedsCleanup = 2,
}

struct CoordinatorInner<R: CommandRunner> {
    runtime: CoreRuntime,
    desktop: DesktopSession,
    runner: Arc<R>,
    transition: tokio::sync::Mutex<()>,
    applied: AtomicU8,
    shutdown_tx: watch::Sender<bool>,
}

pub struct SessionCoordinator<R: CommandRunner> {
    inner: Arc<CoordinatorInner<R>>,
}

impl<R: CommandRunner> Clone for SessionCoordinator<R> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<R: CommandRunner> SessionCoordinator<R> {
    pub fn new(runtime: CoreRuntime, desktop: DesktopSession, runner: Arc<R>) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            inner: Arc::new(CoordinatorInner {
                runtime,
                desktop,
                runner,
                transition: tokio::sync::Mutex::new(()),
                applied: AtomicU8::new(AppliedState::NeedsCleanup as u8),
                shutdown_tx,
            }),
        }
    }

    pub fn applied_state(&self) -> AppliedState {
        match self.inner.applied.load(Ordering::Acquire) {
            value if value == AppliedState::Running as u8 => AppliedState::Running,
            value if value == AppliedState::NeedsCleanup as u8 => AppliedState::NeedsCleanup,
            _ => AppliedState::Stopped,
        }
    }

    pub fn request_shutdown(&self) {
        self.inner.shutdown_tx.send_replace(true);
    }

    pub async fn force_stop_all(&self) -> Result<()> {
        let _transition = self.inner.transition.lock().await;
        self.stop_all_locked().await
    }

    pub async fn reconcile(&self, desired_running: bool) -> Result<()> {
        let _transition = self.inner.transition.lock().await;
        if self.inner.desktop == DesktopSession::Unsupported {
            if self.applied_state() == AppliedState::NeedsCleanup {
                self.stop_all_locked().await?;
            } else {
                self.inner.runtime.clear_game().await;
            }
            self.set_applied(AppliedState::Stopped);
            return Ok(());
        }

        if self.applied_state() == AppliedState::NeedsCleanup {
            self.stop_all_locked().await?;
        }

        match (self.applied_state(), desired_running) {
            (AppliedState::Stopped, true) => self.start().await,
            (AppliedState::Running, false) => self.stop_all_locked().await,
            (AppliedState::Stopped, false) | (AppliedState::Running, true) => Ok(()),
            (AppliedState::NeedsCleanup, _) => unreachable!("cleanup was handled above"),
        }
    }

    async fn start(&self) -> Result<()> {
        for &command in start_command_plan(self.inner.desktop) {
            if let Err(start_error) = self.run_command(command).await {
                self.set_applied(AppliedState::NeedsCleanup);
                let cleanup_error = self.stop_all_locked().await.err();
                return match cleanup_error {
                    Some(cleanup_error) => Err(anyhow!(
                        "failed to start session: {start_error:#}; cleanup also failed: {cleanup_error:#}"
                    )),
                    None => Err(start_error).context("failed to start session; cleanup completed"),
                };
            }
        }
        self.set_applied(AppliedState::Running);
        Ok(())
    }

    async fn stop_all_locked(&self) -> Result<()> {
        self.inner.runtime.clear_game().await;
        let mut failures = Vec::new();
        for &command in stop_command_plan(self.inner.desktop) {
            if let Err(error) = self.run_command(command).await {
                failures.push(format!("{command}: {error:#}"));
            }
        }
        if failures.is_empty() {
            self.set_applied(AppliedState::Stopped);
            Ok(())
        } else {
            self.set_applied(AppliedState::NeedsCleanup);
            Err(anyhow!("session cleanup failed: {}", failures.join("; ")))
        }
    }

    async fn run_command(&self, command: SessionCommand) -> Result<()> {
        self.inner.runner.run(command).await
    }

    fn set_applied(&self, state: AppliedState) {
        self.inner.applied.store(state as u8, Ordering::Release);
    }

    fn shutdown_receiver(&self) -> watch::Receiver<bool> {
        self.inner.shutdown_tx.subscribe()
    }

    fn cancel_commands(&self) {
        self.inner.runner.cancel();
    }

    fn reset_command_cancellation(&self) {
        self.inner.runner.reset_cancellation();
    }
}

pub async fn run_session_coordinator<R: CommandRunner>(
    coordinator: SessionCoordinator<R>,
    desired_running: watch::Receiver<bool>,
) -> Result<()> {
    run_session_coordinator_with_retry(coordinator, desired_running, RECONCILIATION_RETRY_DELAY)
        .await
}

pub(crate) async fn run_session_coordinator_with_retry<R: CommandRunner>(
    coordinator: SessionCoordinator<R>,
    mut desired_running: watch::Receiver<bool>,
    retry_delay: Duration,
) -> Result<()> {
    let mut shutdown = coordinator.shutdown_receiver();
    let mut desired = *desired_running.borrow_and_update();

    loop {
        let transition_failed = if let Err(error) = coordinator.reconcile(desired).await {
            eprintln!("OverCrow session transition failed: {error:#}");
            true
        } else {
            false
        };

        if *shutdown.borrow_and_update() {
            return coordinator.reconcile(false).await;
        }

        match desired_running.has_changed() {
            Ok(true) => {
                desired = *desired_running.borrow_and_update();
                continue;
            }
            Err(_) => return coordinator.reconcile(false).await,
            Ok(false) => {}
        }

        if transition_failed {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow_and_update() {
                        return coordinator.reconcile(false).await;
                    }
                }
                changed = desired_running.changed() => {
                    if changed.is_err() {
                        return coordinator.reconcile(false).await;
                    }
                    desired = *desired_running.borrow_and_update();
                }
                () = tokio::time::sleep(retry_delay) => {}
            }
            continue;
        }

        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow_and_update() {
                    return coordinator.reconcile(false).await;
                }
            }
            changed = desired_running.changed() => {
                if changed.is_err() {
                    return coordinator.reconcile(false).await;
                }
                desired = *desired_running.borrow_and_update();
            }
        }
    }
}

pub async fn shutdown_session_coordinator<R: CommandRunner>(
    coordinator: &SessionCoordinator<R>,
    task: Option<&mut JoinHandle<Result<()>>>,
    timeout: Duration,
) -> Result<()> {
    shutdown_session_coordinator_with_timeouts(
        coordinator,
        task,
        timeout,
        SHUTDOWN_CANCEL_GRACE,
        FORCED_CLEANUP_TIMEOUT,
    )
    .await
}

pub(crate) async fn shutdown_session_coordinator_with_timeouts<R: CommandRunner>(
    coordinator: &SessionCoordinator<R>,
    task: Option<&mut JoinHandle<Result<()>>>,
    timeout: Duration,
    cancel_grace: Duration,
    forced_cleanup_timeout: Duration,
) -> Result<()> {
    coordinator.request_shutdown();
    let mut shutdown_error = None;
    if let Some(task) = task {
        match tokio::time::timeout(timeout, &mut *task).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                shutdown_error = Some(error.context("session coordinator cleanup failed"));
            }
            Ok(Err(error)) => {
                shutdown_error = Some(anyhow!("session coordinator task failed: {error}"));
            }
            Err(_) => {
                shutdown_error = Some(anyhow!("session coordinator shutdown timed out"));
                coordinator.cancel_commands();
                if tokio::time::timeout(cancel_grace, &mut *task)
                    .await
                    .is_err()
                {
                    task.abort();
                    let _ = task.await;
                }
            }
        }
    }

    coordinator.reset_command_cancellation();
    let cleanup_coordinator = coordinator.clone();
    let mut forced_cleanup_task =
        tokio::spawn(async move { cleanup_coordinator.force_stop_all().await });
    let forced_cleanup =
        tokio::time::timeout(forced_cleanup_timeout, &mut forced_cleanup_task).await;
    let cleanup_error = match forced_cleanup {
        Ok(Ok(Ok(()))) => None,
        Ok(Ok(Err(error))) => Some(error.context("forced session cleanup failed")),
        Ok(Err(error)) => Some(anyhow!("forced session cleanup task failed: {error}")),
        Err(_) => {
            coordinator.cancel_commands();
            let joined = tokio::time::timeout(cancel_grace, &mut forced_cleanup_task).await;
            if joined.is_err() {
                forced_cleanup_task.abort();
                let _ = forced_cleanup_task.await;
            }
            Some(anyhow!("forced session cleanup timed out"))
        }
    };

    match (shutdown_error, cleanup_error) {
        (None, None) => Ok(()),
        (Some(shutdown), None) => Err(shutdown),
        (None, Some(cleanup)) => Err(cleanup),
        (Some(shutdown), Some(cleanup)) => Err(anyhow!(
            "{shutdown:#}; subsequent forced cleanup also failed: {cleanup:#}"
        )),
    }
}
