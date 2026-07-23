use std::time::Duration;

use chrono::{Local, TimeZone, Timelike};
use overcrow_protocol::GameTelemetry;

use super::{ClockPresentation, PerformancePresentation};

fn telemetry(cpu_percent_hundredths: u32, resident_bytes: u64) -> GameTelemetry {
    GameTelemetry {
        cpu_percent_hundredths: Some(cpu_percent_hundredths),
        resident_bytes: Some(resident_bytes),
        ..GameTelemetry::default()
    }
}

#[test]
fn performance_uses_explicit_unavailable_markers() {
    let view = PerformancePresentation::from(None);

    assert_eq!(view.cpu, "—");
    assert_eq!(view.ram, "—");
    assert_eq!(view.host_cpu_temperature, "—");
    assert_eq!(view.host_gpu_temperature, "—");
}

#[test]
fn cpu_hundredths_and_binary_ram_are_formatted_deterministically() {
    let view = PerformancePresentation::from(Some(telemetry(12_345, 3 * 1024 * 1024 * 1024)));

    assert_eq!(view.cpu, "123.45%");
    assert_eq!(view.ram, "3.00 GiB");
}

#[test]
fn host_temperatures_are_formatted_separately() {
    let view = PerformancePresentation::from(Some(GameTelemetry {
        cpu_temperature_millicelsius: Some(62_345),
        gpu_temperature_millicelsius: Some(70_000),
        ..GameTelemetry::default()
    }));

    assert_eq!(view.host_cpu_temperature, "62.3 °C");
    assert_eq!(view.host_gpu_temperature, "70.0 °C");
}

#[test]
fn clock_formats_local_time_and_repaints_at_the_next_minute() {
    let now = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 8, 42)
        .single()
        .expect("the fixed local timestamp should be unambiguous")
        .with_nanosecond(250_000_000)
        .expect("the fixed nanoseconds should be valid");

    let view = ClockPresentation::from(now);

    assert_eq!(view.time, "14:08");
    assert_eq!(view.date, "17/07/2026");
    assert_eq!(view.repaint_after, Duration::from_millis(17_750));
}

#[test]
fn clock_at_an_exact_minute_repaints_after_a_full_minute() {
    let now = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 9, 0)
        .single()
        .expect("the fixed local timestamp should be unambiguous");

    assert_eq!(
        ClockPresentation::from(now).repaint_after,
        Duration::from_secs(60)
    );
}

#[test]
fn clock_one_nanosecond_before_a_minute_repaints_after_one_nanosecond() {
    let now = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 8, 59)
        .single()
        .expect("the fixed local timestamp should be unambiguous")
        .with_nanosecond(999_999_999)
        .expect("the fixed nanoseconds should be valid");

    assert_eq!(
        ClockPresentation::from(now).repaint_after,
        Duration::from_nanos(1)
    );
}
