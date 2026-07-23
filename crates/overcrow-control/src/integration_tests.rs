use std::{
    fs,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::integration::{
    ExitObservation, INTEGRATION_COMMAND_TIMEOUT, IntegrationCommandRunner, IntegrationController,
    LinuxNonReapingExitObserver, SystemIntegrationCommandRunner, run_bounded_command,
    run_bounded_command_observed, trusted_helper_path,
};

#[derive(Default)]
struct RecordingRunner(Mutex<Vec<(PathBuf, Vec<&'static str>)>>);

impl IntegrationCommandRunner for RecordingRunner {
    fn run(&self, program: &std::path::Path, args: &'static [&'static str]) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .push((program.to_owned(), args.to_vec()));
        Ok(())
    }
}

#[test]
fn trusted_system_layout_has_one_fixed_helper() {
    assert_eq!(
        trusted_helper_path(
            std::path::Path::new("/usr/bin/overcrow-control"),
            Some(std::path::Path::new("/home/alice")),
        )
        .unwrap(),
        PathBuf::from("/usr/lib/overcrow/overcrow-integrate")
    );
}

#[test]
fn trusted_local_layout_is_bound_to_the_exact_home() {
    assert_eq!(
        trusted_helper_path(
            std::path::Path::new("/home/alice/.local/bin/overcrow-control"),
            Some(std::path::Path::new("/home/alice")),
        )
        .unwrap(),
        PathBuf::from("/home/alice/.local/lib/overcrow/overcrow-integrate")
    );
    assert!(
        trusted_helper_path(
            std::path::Path::new("/tmp/overcrow-control"),
            Some(std::path::Path::new("/home/alice")),
        )
        .is_err()
    );
}

#[test]
fn ensure_ready_uses_only_the_fixed_enable_argument() {
    let runner = Arc::new(RecordingRunner::default());
    let controller = IntegrationController::injected(
        PathBuf::from("/trusted/overcrow-integrate"),
        runner.clone(),
    );

    controller.ensure_ready().unwrap();

    assert_eq!(
        *runner.0.lock().unwrap(),
        vec![(PathBuf::from("/trusted/overcrow-integrate"), vec!["enable"])]
    );
}

#[test]
fn system_runner_executes_the_selected_program_directly() {
    SystemIntegrationCommandRunner
        .run(std::path::Path::new("/usr/bin/true"), &["enable"])
        .unwrap();
}

#[test]
fn integration_timeout_is_one_explicit_minute() {
    assert_eq!(INTEGRATION_COMMAND_TIMEOUT, Duration::from_secs(60));
}

#[test]
fn command_timeout_kills_and_reaps_the_child_before_returning() {
    let started = Instant::now();
    let error = run_bounded_command("/bin/sleep", &["10"], Duration::from_millis(30)).unwrap_err();

    assert!(
        error.contains("timed out"),
        "unexpected runner error: {error}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

fn process_exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal zero checks process existence without delivering a signal.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

fn assert_process_disappears(pid: libc::pid_t) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_exists(pid) && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!process_exists(pid), "process {pid} survived group cleanup");
}

#[test]
fn command_timeout_kills_the_dedicated_group_including_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("descendant.pid");
    let script: &'static str =
        Box::leak(format!("sleep 10 & echo $! > '{}'; wait", pid_file.display()).into_boxed_str());
    let args: &'static [&'static str] = Box::leak(vec!["-c", script].into_boxed_slice());
    let leader = Arc::new(Mutex::new(None));
    let observed = leader.clone();

    let error =
        run_bounded_command_observed("/bin/sh", args, Duration::from_millis(300), move |pid| {
            *observed.lock().unwrap() = Some(pid)
        })
        .unwrap_err();

    assert!(error.contains("timed out"));
    let leader = leader.lock().unwrap().expect("leader PID was observed") as libc::pid_t;
    let descendant = fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse::<libc::pid_t>()
        .unwrap();
    assert_ne!(leader, descendant);
    assert_process_disappears(leader);
    assert_process_disappears(descendant);
}

#[test]
fn recovery_cleanup_timeout_prevents_a_later_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("cleanup-descendant.pid");
    let mutation = temp.path().join("mutation-after-cleanup");
    let script: &'static str = Box::leak(
        format!(
            "(sleep 2; touch '{}') & echo $! > '{}'; wait",
            mutation.display(),
            pid_file.display()
        )
        .into_boxed_str(),
    );
    let args: &'static [&'static str] = Box::leak(vec!["-c", script].into_boxed_slice());

    let error = run_bounded_command_observed("/bin/sh", args, Duration::from_millis(300), |_| {})
        .unwrap_err();

    assert!(error.contains("timed out"));
    let descendant = fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse::<libc::pid_t>()
        .unwrap();
    assert_process_disappears(descendant);
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        !mutation.exists(),
        "a killed cleanup group performed a later mutation"
    );
}

fn leader_exit_with_descendant(exit_code: u8) -> (Result<(), String>, libc::pid_t) {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("descendant.pid");
    let script: &'static str = Box::leak(
        format!(
            "sleep 10 & echo $! > '{}'; exit {exit_code}",
            pid_file.display()
        )
        .into_boxed_str(),
    );
    let args: &'static [&'static str] = Box::leak(vec!["-c", script].into_boxed_slice());
    let result = run_bounded_command_observed("/bin/sh", args, Duration::from_secs(2), |_| {});
    let descendant = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse::<libc::pid_t>()
        .unwrap();
    (result, descendant)
}

#[test]
fn successful_leader_exit_still_cleans_a_surviving_group_descendant() {
    let (result, descendant) = leader_exit_with_descendant(0);
    assert_eq!(result, Ok(()));
    assert_process_disappears(descendant);
}

#[test]
fn failed_leader_exit_preserves_status_and_cleans_a_surviving_group_descendant() {
    let (result, descendant) = leader_exit_with_descendant(7);
    assert!(result.unwrap_err().contains("exit status: 7"));
    assert_process_disappears(descendant);
}

#[test]
fn linux_exit_observation_keeps_pid_pinned_until_explicit_reap() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 7"]).process_group(0);
    let mut child = command.spawn().unwrap();
    let pid = child.id() as libc::pid_t;
    let observer = LinuxNonReapingExitObserver;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match observer.observe(pid).unwrap() {
            ExitObservation::Exited => break,
            ExitObservation::Running => {
                assert!(Instant::now() < deadline, "exit observation timed out");
                std::thread::yield_now();
            }
        }
    }

    assert!(
        process_exists(pid),
        "non-reaping observation released the PID"
    );
    assert_eq!(child.wait().unwrap().code(), Some(7));
    assert!(
        !process_exists(pid),
        "explicit wait did not reap the leader"
    );
}
