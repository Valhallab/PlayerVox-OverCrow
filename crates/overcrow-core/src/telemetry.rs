use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use overcrow_protocol::GameTelemetry;

use crate::{ProcessInfo, ProcessResources, classify_process_identity};

const MAX_ANCESTORS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessGroupIdentity {
    Steam(u32),
    Native,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessGroupKey {
    active_pid: u32,
    active_start_ticks: u64,
    identity: ProcessGroupIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessSample {
    key: ProcessGroupKey,
    resources: ProcessResources,
    observed_at: Instant,
}

impl ProcessSample {
    pub(crate) fn new(
        active_pid: u32,
        active_start_ticks: u64,
        steam_app_id: Option<u32>,
        resources: ProcessResources,
        observed_at: Instant,
    ) -> Self {
        Self {
            key: ProcessGroupKey {
                active_pid,
                active_start_ticks,
                identity: steam_app_id
                    .map_or(ProcessGroupIdentity::Native, ProcessGroupIdentity::Steam),
            },
            resources,
            observed_at,
        }
    }

    #[cfg(test)]
    pub(crate) fn resources(self) -> ProcessResources {
        self.resources
    }
}

#[derive(Debug, Default)]
pub struct TelemetrySampler {
    previous: Option<ProcessSample>,
}

impl TelemetrySampler {
    pub(crate) fn observe(
        &mut self,
        sample: ProcessSample,
        ticks_per_second: u64,
    ) -> GameTelemetry {
        let cpu_percent_hundredths = self.previous.and_then(|previous| {
            (previous.key == sample.key)
                .then(|| cpu_percent_hundredths(previous, sample, ticks_per_second))?
        });
        self.previous = Some(sample);

        GameTelemetry {
            cpu_percent_hundredths,
            resident_bytes: Some(sample.resources.resident_bytes),
            ..GameTelemetry::default()
        }
    }

    pub(crate) fn reset(&mut self) {
        self.previous = None;
    }
}

pub(crate) fn collect_process_sample(
    active_pid: u32,
    steam_app_id: Option<u32>,
    processes: &HashMap<u32, ProcessInfo>,
    observed_at: Instant,
) -> Option<ProcessSample> {
    let active = processes.get(&active_pid)?;
    let resources = match steam_app_id {
        Some(steam_app_id) => (classify_process_identity(active_pid, processes).steam_app_id
            == Some(steam_app_id))
        .then(|| {
            aggregate_resources(processes.values().filter(|process| {
                classify_process_identity(process.pid, processes).steam_app_id == Some(steam_app_id)
            }))
        })?,
        None => aggregate_resources(
            processes
                .values()
                .filter(|process| is_descendant_or_self(process.pid, active_pid, processes)),
        ),
    };

    Some(ProcessSample::new(
        active_pid,
        active.start_ticks,
        steam_app_id,
        resources,
        observed_at,
    ))
}

fn aggregate_resources<'a>(processes: impl Iterator<Item = &'a ProcessInfo>) -> ProcessResources {
    processes.fold(ProcessResources::default(), |total, process| {
        ProcessResources {
            total_cpu_ticks: total
                .total_cpu_ticks
                .saturating_add(process.resources.total_cpu_ticks),
            resident_bytes: total
                .resident_bytes
                .saturating_add(process.resources.resident_bytes),
        }
    })
}

fn is_descendant_or_self(pid: u32, active_pid: u32, processes: &HashMap<u32, ProcessInfo>) -> bool {
    let mut next_pid = pid;
    let mut seen = HashSet::new();

    for _ in 0..MAX_ANCESTORS {
        if !seen.insert(next_pid) {
            return false;
        }
        if next_pid == active_pid {
            return true;
        }
        let Some(process) = processes.get(&next_pid) else {
            return false;
        };
        next_pid = process.parent_pid;
    }

    false
}

fn cpu_percent_hundredths(
    previous: ProcessSample,
    current: ProcessSample,
    ticks_per_second: u64,
) -> Option<u32> {
    let elapsed_nanos = current
        .observed_at
        .checked_duration_since(previous.observed_at)?
        .as_nanos();
    let elapsed_ticks = elapsed_nanos.checked_mul(u128::from(ticks_per_second))?;
    if elapsed_ticks == 0 {
        return None;
    }
    let tick_delta = current
        .resources
        .total_cpu_ticks
        .checked_sub(previous.resources.total_cpu_ticks)?;
    let hundredths = u128::from(tick_delta)
        .saturating_mul(10_000)
        .saturating_mul(1_000_000_000)
        / elapsed_ticks;
    Some(u32::try_from(hundredths).unwrap_or(u32::MAX))
}
