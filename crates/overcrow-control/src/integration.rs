use std::{
    env, fs, io,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const INTEGRATION_ARGUMENTS: &[&str] = &["enable"];
pub(crate) const INTEGRATION_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(10);
const COMMAND_GROUP_TERM_GRACE: Duration = Duration::from_millis(100);

pub trait IntegrationSetup: Send + Sync {
    fn ensure_ready(&self) -> Result<(), String>;
}

pub trait IntegrationCommandRunner: Send + Sync {
    fn run(&self, program: &Path, args: &'static [&'static str]) -> Result<(), String>;
}

pub struct SystemIntegrationCommandRunner;

impl IntegrationCommandRunner for SystemIntegrationCommandRunner {
    fn run(&self, program: &Path, args: &'static [&'static str]) -> Result<(), String> {
        run_bounded_command(program, args, INTEGRATION_COMMAND_TIMEOUT)
            .map_err(|error| format!("integration helper failed: {error}"))
    }
}

pub struct IntegrationController {
    helper: PathBuf,
    runner: Arc<dyn IntegrationCommandRunner>,
}

impl IntegrationController {
    pub fn from_current_process() -> Result<Self, String> {
        let executable = env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(|error| format!("could not identify the installed control binary: {error}"))?;
        let home = env::var_os("HOME").map(PathBuf::from);
        let helper = trusted_helper_path(&executable, home.as_deref())?;
        validate_helper(&helper)?;
        Ok(Self::injected(
            helper,
            Arc::new(SystemIntegrationCommandRunner),
        ))
    }

    pub fn injected(helper: PathBuf, runner: Arc<dyn IntegrationCommandRunner>) -> Self {
        Self { helper, runner }
    }

    pub fn ensure_ready(&self) -> Result<(), String> {
        self.runner.run(&self.helper, INTEGRATION_ARGUMENTS)
    }
}

impl IntegrationSetup for IntegrationController {
    fn ensure_ready(&self) -> Result<(), String> {
        Self::ensure_ready(self)
    }
}

pub fn trusted_helper_path(executable: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    if !is_normal_absolute_path(executable) {
        return Err("control binary path is not an absolute normalized path".to_owned());
    }
    if is_trusted_system_control_binary(executable) {
        return Ok(PathBuf::from("/usr/lib/overcrow/overcrow-integrate"));
    }

    let Some(home) = home.filter(|path| is_normal_absolute_path(path)) else {
        return Err("an absolute HOME is required for a local installation".to_owned());
    };
    if executable == home.join(".local/bin/overcrow-control") {
        return Ok(home.join(".local/lib/overcrow/overcrow-integrate"));
    }

    Err("control binary is not in a supported installed layout".to_owned())
}

pub(crate) fn is_trusted_system_control_binary(executable: &Path) -> bool {
    executable == Path::new("/usr/bin/overcrow-control")
}

pub(crate) fn is_normal_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

fn validate_helper(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect integration helper: {error}"))?;
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err("integration helper is not a regular executable file".to_owned());
    }
    Ok(())
}

pub(crate) fn run_bounded_command(
    program: impl AsRef<std::ffi::OsStr>,
    args: &'static [&'static str],
    timeout: Duration,
) -> Result<(), String> {
    run_bounded_command_observed(program, args, timeout, |_| {})
}

pub(crate) fn run_bounded_command_observed(
    program: impl AsRef<std::ffi::OsStr>,
    args: &'static [&'static str],
    timeout: Duration,
    on_spawn: impl FnOnce(u32),
) -> Result<(), String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start command: {error}"))?;
    let group = match ProcessGroup::from_child(&child) {
        Ok(group) => group,
        Err(error) => {
            let kill_error = child.kill().err();
            let reap_error = child.wait().err();
            let mut message = error;
            append_cleanup_error(&mut message, "leader kill", kill_error);
            append_cleanup_error(&mut message, "leader reap", reap_error);
            return Err(message);
        }
    };
    on_spawn(child.id());
    let observer = LinuxNonReapingExitObserver;
    let started = Instant::now();
    loop {
        match observer.observe(group.pid()) {
            Ok(ExitObservation::Exited) => {
                return finish_observed_exit(&mut child, group);
            }
            Ok(ExitObservation::Running) if started.elapsed() < timeout => {
                thread::sleep(COMMAND_POLL_INTERVAL);
            }
            Ok(ExitObservation::Running) => {
                return Err(terminate_group_and_reap(
                    &mut child,
                    group,
                    "command timed out",
                ));
            }
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    let reap_error = child.wait().err();
                    let mut message = format!(
                        "lost command-child ownership during non-reaping observation; refusing an unsafe process-group signal: {error}"
                    );
                    append_cleanup_error(&mut message, "leader reap", reap_error);
                    return Err(message);
                }
                return Err(terminate_group_and_reap(
                    &mut child,
                    group,
                    &format!("could not observe command exit without reaping: {error}"),
                ));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExitObservation {
    Running,
    Exited,
}

pub(crate) struct LinuxNonReapingExitObserver;

impl LinuxNonReapingExitObserver {
    pub(crate) fn observe(&self, pid: libc::pid_t) -> io::Result<ExitObservation> {
        let id = libc::id_t::try_from(pid).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "child PID is out of range")
        })?;
        loop {
            // Linux `waitid` with `WNOWAIT` reports exit state while deliberately retaining the
            // zombie. Keeping the direct child unreaped pins both its PID and dedicated PGID until
            // the caller has signalled any remaining group members. The zeroed `siginfo_t` is valid
            // output storage and also guarantees `si_pid() == 0` when `WNOHANG` reports no event.
            let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
            // SAFETY: `pid` names the owned direct child, `info` is writable for the call, and the
            // options request only exited state without consuming it. EINTR is retried below.
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    id,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result == 0 {
                // SAFETY: successful `waitid` initialized the relevant siginfo fields; the entire
                // structure was zero-initialized for the no-event case.
                let observed_pid = unsafe { info.si_pid() };
                return match observed_pid {
                    0 => Ok(ExitObservation::Running),
                    observed if observed == pid => Ok(ExitObservation::Exited),
                    _ => Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "waitid returned an unexpected child PID",
                    )),
                };
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

fn finish_observed_exit(
    child: &mut std::process::Child,
    group: ProcessGroup,
) -> Result<(), String> {
    let cleanup_error = group.kill().err();
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            let mut message = format!("could not reap observed command leader: {error}");
            append_cleanup_error(&mut message, "process-group cleanup", cleanup_error);
            return Err(message);
        }
    };
    if status.success() {
        return match cleanup_error {
            Some(error) => Err(format!(
                "command succeeded but process-group cleanup failed: {error}"
            )),
            None => Ok(()),
        };
    }

    let mut message = format!("command exited with {status}");
    append_cleanup_error(&mut message, "process-group cleanup", cleanup_error);
    Err(message)
}

#[derive(Clone, Copy)]
struct ProcessGroup(libc::pid_t);

impl ProcessGroup {
    fn from_child(child: &std::process::Child) -> Result<Self, String> {
        let pid = libc::pid_t::try_from(child.id())
            .ok()
            .filter(|pid| *pid > 1)
            .ok_or_else(|| "spawned command has an invalid process-group ID".to_owned())?;
        // `process_group(0)` makes the child a leader. A distinct positive child PID cannot
        // identify the caller's process group.
        Ok(Self(pid))
    }

    fn signal(self, signal: libc::c_int) -> io::Result<()> {
        // SAFETY: the negative, validated child PID addresses only its dedicated process group,
        // and callers provide a valid fixed signal value.
        if unsafe { libc::kill(-self.0, signal) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }

    fn terminate(self) -> io::Result<()> {
        self.signal(libc::SIGTERM)
    }

    fn kill(self) -> io::Result<()> {
        self.signal(libc::SIGKILL)
    }

    const fn pid(self) -> libc::pid_t {
        self.0
    }
}

fn terminate_group_and_reap(
    child: &mut std::process::Child,
    group: ProcessGroup,
    reason: &str,
) -> String {
    let terminate_error = group.terminate().err();
    // Keep the leader unreaped during the grace window so its process-group ID cannot be
    // reused before the forced-kill attempt.
    thread::sleep(COMMAND_GROUP_TERM_GRACE);
    let kill_error = group.kill().err();
    let reap_error = child.wait().err();
    let mut message = reason.to_owned();
    append_cleanup_error(&mut message, "process-group termination", terminate_error);
    append_cleanup_error(&mut message, "process-group forced kill", kill_error);
    append_cleanup_error(&mut message, "leader reap", reap_error);
    message
}

fn append_cleanup_error(message: &mut String, operation: &str, error: Option<io::Error>) {
    if let Some(error) = error {
        message.push_str(&format!("; {operation} failed: {error}"));
    }
}
