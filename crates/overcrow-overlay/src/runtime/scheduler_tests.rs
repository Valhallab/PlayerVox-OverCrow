use std::time::{Duration, Instant};

use super::OverlayScheduler;

#[test]
fn warframe_tick_runs_once_per_due_second_not_once_per_frame() {
    let origin = Instant::now();
    let mut scheduler = OverlayScheduler::default();
    assert!(scheduler.take_warframe_tick(true, origin));
    for millis in 1..1_000 {
        assert!(!scheduler.take_warframe_tick(true, origin + Duration::from_millis(millis)));
    }
    assert!(scheduler.take_warframe_tick(true, origin + Duration::from_secs(1)));
}

#[test]
fn disabling_clears_the_warframe_deadline_and_reenable_is_immediately_due() {
    let origin = Instant::now();
    let mut scheduler = OverlayScheduler::default();
    assert!(scheduler.take_warframe_tick(true, origin));
    assert!(!scheduler.take_warframe_tick(false, origin + Duration::from_millis(10)));
    assert!(scheduler.take_warframe_tick(true, origin + Duration::from_millis(20)));
}

#[test]
fn market_refresh_fires_once_per_snapshot_deadline() {
    let origin = Instant::now();
    let mut scheduler = OverlayScheduler::default();

    assert!(!scheduler.take_market_refresh(true, true, 7, 1_005, origin, 1_000));
    assert!(!scheduler.take_market_refresh(
        true,
        true,
        7,
        1_005,
        origin + Duration::from_secs(4),
        1_004,
    ));
    assert!(scheduler.take_market_refresh(
        true,
        true,
        7,
        1_005,
        origin + Duration::from_secs(5),
        1_005,
    ));
    assert!(!scheduler.take_market_refresh(
        true,
        true,
        7,
        1_005,
        origin + Duration::from_secs(6),
        1_006,
    ));

    assert!(!scheduler.take_market_refresh(
        true,
        true,
        8,
        1_010,
        origin + Duration::from_secs(6),
        1_006,
    ));
    assert!(scheduler.take_market_refresh(
        true,
        true,
        8,
        1_010,
        origin + Duration::from_secs(10),
        1_010,
    ));
}

#[test]
fn market_refresh_uses_the_mapped_monotonic_deadline_after_wall_clock_moves_backward() {
    let origin = Instant::now();
    let mut scheduler = OverlayScheduler::default();

    assert!(!scheduler.take_market_refresh(true, true, 9, 1_010, origin, 1_000));
    assert!(scheduler.take_market_refresh(
        true,
        true,
        9,
        1_010,
        origin + Duration::from_secs(10),
        990,
    ));
    assert!(!scheduler.take_market_refresh(
        true,
        true,
        9,
        1_010,
        origin + Duration::from_secs(11),
        991,
    ));
}

#[test]
fn disabled_market_work_clears_eligibility_and_reenable_is_due_immediately() {
    let origin = Instant::now();
    let mut scheduler = OverlayScheduler::default();

    assert!(scheduler.take_market_refresh(true, true, 4, 900, origin, 1_000));
    assert!(!scheduler.take_market_refresh(false, true, 4, 900, origin, 1_000));
    assert!(scheduler.take_market_refresh(
        true,
        true,
        4,
        900,
        origin + Duration::from_millis(1),
        1_000,
    ));
}

#[test]
fn copy_expiry_and_next_repaint_use_explicit_deadlines() {
    let origin = Instant::now();
    let mut scheduler = OverlayScheduler::default();
    assert!(scheduler.take_warframe_tick(true, origin));
    scheduler.set_copy_flash_deadline(Some(origin + Duration::from_millis(200)));

    assert_eq!(
        scheduler.next_repaint_after(origin),
        Some(Duration::from_millis(200))
    );
    assert!(!scheduler.take_copy_flash_expired(origin + Duration::from_millis(199)));
    assert!(scheduler.take_copy_flash_expired(origin + Duration::from_millis(200)));
    assert!(!scheduler.take_copy_flash_expired(origin + Duration::from_millis(201)));
    assert_eq!(
        scheduler.next_repaint_after(origin + Duration::from_millis(200)),
        Some(Duration::from_millis(800))
    );
}
