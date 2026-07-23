use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use anyhow::Context;
use procfs::{
    Current, ProcResult, Uptime,
    process::{Process, all_processes},
};

use crate::classifier::{ProcessInfo, ProcessResources, ProcessTiming};

pub fn scan_processes() -> anyhow::Result<HashMap<u32, ProcessInfo>> {
    let daemon = Process::myself().context("failed to inspect the OverCrow daemon process")?;
    let daemon_uid = daemon_uid(daemon.uid())?;
    let observed_at = Instant::now();
    let process_clock = Uptime::current()
        .ok()
        .map(|uptime| (uptime.uptime_duration(), procfs::ticks_per_second()));
    let mut processes = HashMap::new();

    for process in all_processes()? {
        let Ok(process) = process else {
            continue;
        };
        if !process_is_owned_by(daemon_uid, process.uid()) {
            continue;
        }
        let Ok(stat) = process.stat() else {
            continue;
        };
        let (Ok(pid), Ok(parent_pid)) = (u32::try_from(stat.pid), u32::try_from(stat.ppid)) else {
            continue;
        };
        let Some(resources) =
            process_resources(stat.utime, stat.stime, stat.rss, procfs::page_size())
        else {
            continue;
        };

        let environment = process
            .environ()
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect();

        processes.insert(
            pid,
            ProcessInfo {
                pid,
                parent_pid,
                start_ticks: stat.starttime,
                timing: process_clock
                    .and_then(|(uptime, ticks_per_second)| {
                        elapsed_at_observation(stat.starttime, uptime, ticks_per_second)
                    })
                    .map(|elapsed| ProcessTiming::new(elapsed, observed_at)),
                resources,
                name: stat.comm,
                environment,
                command_line: process.cmdline().unwrap_or_default(),
                executable: process.exe().ok(),
            },
        );
    }

    Ok(processes)
}

fn process_resources(
    user_ticks: u64,
    system_ticks: u64,
    rss_pages: u64,
    page_size: u64,
) -> Option<ProcessResources> {
    Some(ProcessResources {
        total_cpu_ticks: user_ticks.saturating_add(system_ticks),
        resident_bytes: rss_pages.checked_mul(page_size)?,
    })
}

fn elapsed_at_observation(
    start_ticks: u64,
    uptime: Duration,
    ticks_per_second: u64,
) -> Option<Duration> {
    let start = (ticks_per_second != 0)
        .then(|| Duration::from_secs_f64(start_ticks as f64 / ticks_per_second as f64))?;
    uptime.checked_sub(start)
}

fn daemon_uid(uid: ProcResult<u32>) -> anyhow::Result<u32> {
    uid.context("failed to read OverCrow daemon UID")
}

fn process_is_owned_by(daemon_uid: u32, process_uid: ProcResult<u32>) -> bool {
    matches!(process_uid, Ok(process_uid) if process_uid == daemon_uid)
}

#[cfg(test)]
mod tests {
    use procfs::ProcError;
    use std::time::{Duration, Instant};

    use super::{
        daemon_uid, elapsed_at_observation, process_is_owned_by, process_resources, scan_processes,
    };
    use crate::{ProcessResources, ProcessTiming};

    #[test]
    fn process_resources_saturate_cpu_ticks_and_check_rss_conversion() {
        assert_eq!(
            process_resources(u64::MAX, 1, 3, 4096),
            Some(ProcessResources {
                total_cpu_ticks: u64::MAX,
                resident_bytes: 12_288,
            })
        );
        assert_eq!(process_resources(1, 2, u64::MAX, 4096), None);
    }

    #[test]
    fn process_age_uses_ticks_since_boot() {
        assert_eq!(
            elapsed_at_observation(12_500, Duration::from_secs(200), 100),
            Some(Duration::from_secs(75))
        );
    }

    #[test]
    fn invalid_start_time_is_unavailable() {
        assert_eq!(
            elapsed_at_observation(20_001, Duration::from_secs(200), 100),
            None
        );
        assert_eq!(
            elapsed_at_observation(10, Duration::from_secs(200), 0),
            None
        );
    }

    #[test]
    fn timing_advances_from_its_observation() {
        let now = Instant::now();
        let timing = ProcessTiming::new(Duration::from_secs(75), now);

        assert_eq!(
            timing.elapsed_at(now + Duration::from_secs(5)),
            Duration::from_secs(80)
        );
    }

    #[test]
    fn uid_policy_includes_only_a_readable_matching_uid() {
        assert!(process_is_owned_by(1000, Ok(1000)));
        assert!(!process_is_owned_by(1000, Ok(1001)));
        assert!(!process_is_owned_by(
            1000,
            Err(ProcError::Other("UID unavailable".to_owned()))
        ));
    }

    #[test]
    fn daemon_uid_errors_instead_of_scanning_without_an_owner() {
        let error = daemon_uid(Err(ProcError::Other("UID unavailable".to_owned())))
            .expect_err("an unreadable daemon UID must abort the scan");

        assert!(
            error
                .to_string()
                .contains("failed to read OverCrow daemon UID")
        );
    }

    #[test]
    fn scan_includes_the_current_process() {
        let processes = scan_processes().expect("/proc should be readable on Linux");
        let pid = std::process::id();
        let current = processes
            .get(&pid)
            .expect("the current test process should be present during its own scan");

        assert_eq!(current.pid, pid);
        assert!(!current.name.is_empty());
        assert!(current.start_ticks > 0);
        assert!(current.timing.is_some());
    }
}
