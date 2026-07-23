use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use overcrow_config::LifecycleSettings;
use overcrow_protocol::{CoreState, GameTelemetry};
use tokio::sync::RwLock;

use crate::{
    CoreRuntime, ProcessInfo, ProcessResources, TelemetrySampler, TemperatureSnapshot,
    WindowObservation, collect_process_sample, telemetry::ProcessSample,
};

fn at(seconds: u64) -> Instant {
    Instant::now() + Duration::from_secs(seconds)
}

fn sample(total_cpu_ticks: u64, resident_bytes: u64, observed_at: Instant) -> ProcessSample {
    ProcessSample::new(
        42,
        1_000,
        Some(620),
        ProcessResources {
            total_cpu_ticks,
            resident_bytes,
        },
        observed_at,
    )
}

fn process(
    pid: u32,
    parent_pid: u32,
    start_ticks: u64,
    steam_app_id: Option<u32>,
    total_cpu_ticks: u64,
    resident_bytes: u64,
) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent_pid,
        start_ticks,
        timing: None,
        resources: ProcessResources {
            total_cpu_ticks,
            resident_bytes,
        },
        name: format!("process-{pid}"),
        environment: steam_app_id
            .map(|id| HashMap::from([("SteamAppId".to_owned(), id.to_string())]))
            .unwrap_or_default(),
        command_line: Vec::new(),
        executable: Some(PathBuf::from(format!("/games/process-{pid}"))),
    }
}

fn processes<const N: usize>(items: [ProcessInfo; N]) -> HashMap<u32, ProcessInfo> {
    items.into_iter().map(|item| (item.pid, item)).collect()
}

#[test]
fn steam_group_uses_each_processes_nearest_nonzero_app_id() {
    let observed_at = at(0);
    let grouped = processes([
        process(10, 1, 100, Some(620), 10, 100),
        process(20, 10, 200, None, 20, 200),
        process(30, 20, 300, Some(730), 40, 400),
        process(40, 30, 400, None, 80, 800),
    ]);

    let result = collect_process_sample(20, Some(620), &grouped, observed_at).unwrap();

    assert_eq!(result.resources().total_cpu_ticks, 30);
    assert_eq!(result.resources().resident_bytes, 300);
}

#[test]
fn native_group_includes_only_the_active_process_and_bounded_descendants() {
    let observed_at = at(0);
    let grouped = processes([
        process(10, 1, 100, None, 10, 100),
        process(20, 10, 200, None, 20, 200),
        process(30, 20, 300, None, 40, 400),
        process(40, 1, 400, None, 80, 800),
    ]);

    let result = collect_process_sample(10, None, &grouped, observed_at).unwrap();

    assert_eq!(result.resources().total_cpu_ticks, 70);
    assert_eq!(result.resources().resident_bytes, 700);
}

#[test]
fn native_descendant_walk_stops_after_sixty_four_processes() {
    let mut grouped = HashMap::new();
    grouped.insert(1, process(1, 0, 1, None, 1, 1));
    for pid in 2..=66 {
        grouped.insert(pid, process(pid, pid - 1, u64::from(pid), None, 1, 1));
    }

    let result = collect_process_sample(1, None, &grouped, at(0)).unwrap();

    assert_eq!(result.resources().total_cpu_ticks, 64);
    assert_eq!(result.resources().resident_bytes, 64);
}

#[test]
fn one_busy_logical_cpu_is_one_hundred_percent() {
    let now = at(0);
    let mut sampler = TelemetrySampler::default();
    assert_eq!(
        sampler
            .observe(sample(100, 10, now), 100)
            .cpu_percent_hundredths,
        None
    );
    assert_eq!(
        sampler
            .observe(sample(300, 20, now + Duration::from_secs(2)), 100)
            .cpu_percent_hundredths,
        Some(10_000)
    );
}

#[test]
fn first_sample_has_ram_but_no_cpu_delta() {
    let mut sampler = TelemetrySampler::default();

    let result = sampler.observe(sample(100, 4096, at(0)), 100);

    assert_eq!(
        result,
        GameTelemetry {
            cpu_percent_hundredths: None,
            resident_bytes: Some(4096),
            ..GameTelemetry::default()
        }
    );
}

#[test]
fn reused_active_pid_resets_the_cpu_baseline() {
    let now = at(0);
    let mut sampler = TelemetrySampler::default();
    sampler.observe(sample(100, 10, now), 100);
    let replacement = ProcessSample::new(
        42,
        2_000,
        Some(620),
        ProcessResources {
            total_cpu_ticks: 10_000,
            resident_bytes: 20,
        },
        now + Duration::from_secs(2),
    );

    let result = sampler.observe(replacement, 100);

    assert_eq!(result.cpu_percent_hundredths, None);
    assert_eq!(result.resident_bytes, Some(20));
}

#[test]
fn changed_group_identity_resets_the_cpu_baseline() {
    let now = at(0);
    let mut sampler = TelemetrySampler::default();
    sampler.observe(sample(100, 10, now), 100);
    let other_group = ProcessSample::new(
        42,
        1_000,
        Some(730),
        ProcessResources {
            total_cpu_ticks: 10_000,
            resident_bytes: 20,
        },
        now + Duration::from_secs(2),
    );

    assert_eq!(
        sampler.observe(other_group, 100).cpu_percent_hundredths,
        None
    );
}

#[test]
fn resource_aggregation_saturates_cpu_ticks_and_rss() {
    let grouped = processes([
        process(10, 1, 100, Some(620), u64::MAX, u64::MAX),
        process(20, 10, 200, None, 1, 1),
    ]);

    let result = collect_process_sample(10, Some(620), &grouped, at(0)).unwrap();

    assert_eq!(result.resources().total_cpu_ticks, u64::MAX);
    assert_eq!(result.resources().resident_bytes, u64::MAX);
}

#[test]
fn missing_active_process_makes_process_resources_unavailable() {
    let grouped = processes([process(10, 1, 100, Some(620), 10, 100)]);

    assert!(collect_process_sample(99, Some(620), &grouped, at(0)).is_none());
}

#[tokio::test]
async fn runtime_snapshot_events_publish_telemetry_changes() {
    let now = at(0);
    let initial = process(42, 1, 1_000, Some(620), 100, 4_096);
    let runtime = CoreRuntime::with_settings(
        std::sync::Arc::new(RwLock::new(CoreState::default())),
        processes([initial]),
        LifecycleSettings {
            enabled: true,
            selected_steam_app_ids: [620].into_iter().collect(),
            ..LifecycleSettings::default()
        },
    )
    .await;
    runtime
        .apply_x11_observation(Some(WindowObservation {
            pid: Some(42),
            app_id: Some("portal2".to_owned()),
            title: "Portal 2".to_owned(),
            rect: overcrow_protocol::Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale: 1.0,
            backend: "x11".to_owned(),
        }))
        .await;
    let mut structural = runtime.snapshots();
    let refreshed = process(42, 1, 1_000, Some(620), 300, 8_192);

    runtime
        .install_refresh_snapshot_at(
            processes([refreshed]),
            TemperatureSnapshot {
                cpu_millicelsius: Some(65_000),
                gpu_millicelsius: Some(70_000),
            },
            now + Duration::from_secs(2),
        )
        .await;

    structural.changed().await.expect("telemetry publication");
    assert_eq!(
        structural.borrow_and_update().snapshot.telemetry,
        Some(GameTelemetry {
            cpu_percent_hundredths: None,
            resident_bytes: Some(8_192),
            cpu_temperature_millicelsius: Some(65_000),
            gpu_temperature_millicelsius: Some(70_000),
        })
    );
}
