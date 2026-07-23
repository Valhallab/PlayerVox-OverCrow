use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use overcrow_config::WidgetProfile;
use overcrow_protocol::{CoreSnapshot, GameWindow, ManualStopwatchSnapshot, OverlayMode, Rect};

use super::{
    ManualStopwatchAction, ManualStopwatchClock, ManualStopwatchPresentation,
    manual_stopwatch_repaint_after, route_manual_stopwatch_action,
};

fn snapshot(elapsed_ms: u64, running: bool) -> ManualStopwatchSnapshot {
    ManualStopwatchSnapshot {
        elapsed_ms,
        running,
    }
}

fn core_snapshot(mode: OverlayMode, running: bool) -> CoreSnapshot {
    CoreSnapshot {
        active_game: Some(GameWindow {
            pid: Some(42),
            steam_app_id: Some(620),
            app_id: Some("portal2".to_owned()),
            title: "Portal 2".to_owned(),
            rect: Rect {
                x: 100,
                y: 200,
                width: 1_920,
                height: 1_080,
            },
            scale: 1.0,
            backend: "test".to_owned(),
        }),
        overlay_mode: mode,
        manual_stopwatch: snapshot(1_250, running),
        ..CoreSnapshot::default()
    }
}

#[test]
fn manual_stopwatch_formats_zero_and_exposes_paused_controls() {
    let view = ManualStopwatchPresentation::new(Duration::ZERO, false, OverlayMode::Interactive);

    assert_eq!(view.elapsed, "00:00:00.00");
    assert_eq!(view.status, "PAUSED");
    assert_eq!(view.toggle_label, "Start");
    assert!(view.controls_visible);
}

#[test]
fn manual_stopwatch_running_presentation_exposes_pause_and_shortcuts() {
    let view = ManualStopwatchPresentation::new(
        Duration::from_millis(3_661_250),
        true,
        OverlayMode::Interactive,
    );

    assert_eq!(view.elapsed, "01:01:01.25");
    assert_eq!(view.status, "RUNNING");
    assert_eq!(view.toggle_label, "Pause");
    assert!(view.controls_visible);
    assert_eq!(view.shortcut_footer, Some(("Super+Alt+P", "Super+Alt+R")));
}

#[test]
fn manual_stopwatch_formats_subseconds() {
    use super::format_manual_stopwatch_elapsed;

    assert_eq!(
        format_manual_stopwatch_elapsed(Duration::from_millis(0)),
        "00:00:00.00"
    );
    assert_eq!(
        format_manual_stopwatch_elapsed(Duration::from_millis(9)),
        "00:00:00.00"
    );
    assert_eq!(
        format_manual_stopwatch_elapsed(Duration::from_millis(10)),
        "00:00:00.01"
    );
    assert_eq!(
        format_manual_stopwatch_elapsed(Duration::from_millis(1_999)),
        "00:00:01.99"
    );
}

#[test]
fn manual_stopwatch_passive_presentation_hides_shortcuts() {
    let view = ManualStopwatchPresentation::new(Duration::from_secs(1), true, OverlayMode::Passive);

    assert!(!view.controls_visible);
    assert_eq!(view.shortcut_footer, None);
}

#[test]
fn manual_stopwatch_repaints_only_while_visible_and_running() {
    let now = Instant::now();
    let mut running_clock = ManualStopwatchClock::default();
    running_clock.sync(snapshot(1_250, true), now);
    let interactive = core_snapshot(OverlayMode::Interactive, true);
    let mut profile = WidgetProfile::default();

    assert_eq!(
        manual_stopwatch_repaint_after(&interactive, &profile, &running_clock, now),
        None
    );

    profile.manual_stopwatch.enabled = true;
    let passive = core_snapshot(OverlayMode::Passive, true);
    assert_eq!(
        manual_stopwatch_repaint_after(&passive, &profile, &running_clock, now),
        None
    );

    let mut without_game = interactive.clone();
    without_game.active_game = None;
    assert_eq!(
        manual_stopwatch_repaint_after(&without_game, &profile, &running_clock, now),
        None
    );

    let mut paused_clock = ManualStopwatchClock::default();
    paused_clock.sync(snapshot(1_250, false), now);
    assert_eq!(
        manual_stopwatch_repaint_after(
            &core_snapshot(OverlayMode::Interactive, false),
            &profile,
            &paused_clock,
            now,
        ),
        None
    );

    // 1250ms → next centisecond in 10ms.
    assert_eq!(
        manual_stopwatch_repaint_after(&interactive, &profile, &running_clock, now),
        Some(Duration::from_millis(10))
    );

    profile.manual_stopwatch.show_in_passive = true;
    assert_eq!(
        manual_stopwatch_repaint_after(&passive, &profile, &running_clock, now),
        Some(Duration::from_millis(10))
    );
}

#[test]
fn manual_stopwatch_routes_interactive_mouse_actions() {
    let toggles = Cell::new(0);
    let resets = Cell::new(0);

    route_manual_stopwatch_action(
        OverlayMode::Interactive,
        Some(ManualStopwatchAction::Toggle),
        || toggles.set(toggles.get() + 1),
        || resets.set(resets.get() + 1),
    );
    route_manual_stopwatch_action(
        OverlayMode::Interactive,
        Some(ManualStopwatchAction::Reset),
        || toggles.set(toggles.get() + 1),
        || resets.set(resets.get() + 1),
    );

    assert_eq!(toggles.get(), 1);
    assert_eq!(resets.get(), 1);
}

#[test]
fn manual_stopwatch_passive_mouse_actions_are_read_only() {
    let toggles = Cell::new(0);
    let resets = Cell::new(0);

    route_manual_stopwatch_action(
        OverlayMode::Passive,
        Some(ManualStopwatchAction::Toggle),
        || toggles.set(toggles.get() + 1),
        || resets.set(resets.get() + 1),
    );
    route_manual_stopwatch_action(
        OverlayMode::Passive,
        Some(ManualStopwatchAction::Reset),
        || toggles.set(toggles.get() + 1),
        || resets.set(resets.get() + 1),
    );

    assert_eq!(toggles.get(), 0);
    assert_eq!(resets.get(), 0);
}

#[test]
fn manual_stopwatch_advances_only_while_running() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();

    clock.sync(snapshot(1_250, true), origin);
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(750)),
        Duration::from_secs(2)
    );

    clock.sync(snapshot(2_100, false), origin + Duration::from_secs(1));
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(30)),
        Duration::from_millis(2_100)
    );
}

#[test]
fn manual_stopwatch_resynchronizes_without_pre_anchor_underflow() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();

    clock.sync(snapshot(1_000, true), origin);
    clock.sync(snapshot(5_000, true), origin + Duration::from_secs(2));

    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(3)),
        Duration::from_secs(6)
    );
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(1)),
        Duration::from_secs(5)
    );
}

#[test]
fn manual_stopwatch_running_resync_does_not_rewind_for_transport_jitter() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(1_000, true), origin);

    clock.sync(snapshot(1_900, true), origin + Duration::from_secs(1));

    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(1)),
        Duration::from_secs(2)
    );
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(1_100)),
        Duration::from_millis(2_100)
    );

    clock.sync(snapshot(1_950, true), origin + Duration::from_millis(1_100));
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(1_100)),
        Duration::from_millis(2_100)
    );
}

#[test]
fn manual_stopwatch_accepts_core_reset_as_authoritative() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(10_000, true), origin);

    clock.sync(
        ManualStopwatchSnapshot::default(),
        origin + Duration::from_secs(1),
    );

    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(30)),
        Duration::ZERO
    );
    assert!(!clock.running());
}

#[test]
fn manual_stopwatch_pause_freezes_immediately_without_overshoot() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(1_800, true), origin);

    // User hits pause at the displayed 1.80s — freeze before Core answers.
    clock.apply_local_toggle(origin);
    assert!(!clock.running());
    assert_eq!(clock.elapsed_at(origin), Duration::from_millis(1_800));

    // Time passes while the D-Bus toggle is in flight — must not advance.
    let later = origin + Duration::from_millis(300);
    assert_eq!(clock.elapsed_at(later), Duration::from_millis(1_800));

    // Stale poll still reporting running must not reopen interpolation.
    clock.sync(snapshot(1_750, true), origin + Duration::from_millis(100));
    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(400)),
        Duration::from_millis(1_800)
    );

    // Core confirms pause with its authoritative elapsed.
    clock.sync(snapshot(1_810, false), origin + Duration::from_millis(200));
    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(5)),
        Duration::from_millis(1_810)
    );
}

#[test]
fn manual_stopwatch_start_is_optimistic_and_ignores_stale_paused() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(1_800, false), origin);

    clock.apply_local_toggle(origin);
    assert!(clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(100)),
        Duration::from_millis(1_900)
    );

    // Stale paused sample must not freeze us again mid-start.
    clock.sync(snapshot(1_800, false), origin + Duration::from_millis(50));
    assert!(clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(100)),
        Duration::from_millis(1_900)
    );

    clock.sync(snapshot(1_850, true), origin + Duration::from_millis(50));
    assert!(clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(150)),
        Duration::from_millis(1_950)
    );
}

#[test]
fn manual_stopwatch_reset_freezes_at_zero_immediately() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(5_000, true), origin);

    clock.apply_local_reset(origin + Duration::from_millis(250));
    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(1)),
        Duration::ZERO
    );

    // Stale running sample ignored until Core confirms reset (paused).
    clock.sync(snapshot(5_200, true), origin + Duration::from_millis(300));
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(1)),
        Duration::ZERO
    );

    clock.sync(snapshot(0, false), origin + Duration::from_millis(400));
    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(2)),
        Duration::ZERO
    );
}

#[test]
fn manual_stopwatch_mismatch_is_ignored_only_before_the_optimism_deadline() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(1_000, false), origin);
    clock.apply_local_toggle(origin);

    clock.sync(
        snapshot(1_200, false),
        origin + Duration::from_millis(2_999),
    );
    assert!(clock.running());

    clock.sync(snapshot(1_300, false), origin + Duration::from_secs(3));
    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(30)),
        Duration::from_millis(1_300)
    );
}

#[test]
fn manual_stopwatch_reset_requires_a_paused_zero_acknowledgement() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(5_000, true), origin);
    clock.apply_local_reset(origin);

    // A stale pause from before reset must neither restore elapsed time nor
    // clear the reset expectation.
    clock.sync(snapshot(5_200, false), origin + Duration::from_millis(100));
    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(1)),
        Duration::ZERO
    );
    clock.sync(snapshot(5_300, true), origin + Duration::from_millis(200));
    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(1)),
        Duration::ZERO
    );

    clock.sync(snapshot(0, false), origin + Duration::from_millis(300));
    clock.sync(snapshot(100, true), origin + Duration::from_millis(400));
    assert!(clock.running());
}

#[test]
fn manual_stopwatch_reset_then_start_rejects_stale_prereset_running_sample() {
    let origin = Instant::now();
    let action_at = origin + Duration::from_millis(100);
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(5_000, true), origin);
    clock.apply_local_reset(action_at);
    clock.apply_local_toggle(action_at);

    clock.sync(snapshot(5_100, true), origin + Duration::from_millis(200));

    assert!(clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(300)),
        Duration::from_millis(200)
    );
}

#[test]
fn manual_stopwatch_reset_ack_is_intermediate_before_post_reset_running_ack() {
    let origin = Instant::now();
    let action_at = origin + Duration::from_millis(100);
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(5_000, true), origin);
    clock.apply_local_reset(action_at);
    clock.apply_local_toggle(action_at);

    clock.sync(snapshot(5_100, true), origin + Duration::from_millis(150));
    clock.sync(snapshot(0, false), origin + Duration::from_millis(200));
    assert!(clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(300)),
        Duration::from_millis(200)
    );

    clock.sync(snapshot(150, true), origin + Duration::from_millis(300));
    assert!(clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(400)),
        Duration::from_millis(300)
    );
}

#[test]
fn manual_stopwatch_toggles_after_reset_compose_by_parity() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(5_000, true), origin);
    clock.apply_local_reset(origin + Duration::from_millis(100));
    clock.apply_local_toggle(origin + Duration::from_millis(100));
    clock.apply_local_toggle(origin + Duration::from_millis(200));

    clock.sync(snapshot(5_100, false), origin + Duration::from_millis(300));
    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(400)),
        Duration::from_millis(100)
    );

    clock.apply_local_toggle(origin + Duration::from_millis(400));
    clock.sync(snapshot(5_200, true), origin + Duration::from_millis(500));
    assert!(clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(600)),
        Duration::from_millis(300)
    );

    clock.sync(snapshot(0, false), origin + Duration::from_millis(600));
    clock.apply_local_toggle(origin + Duration::from_millis(650));
    clock.apply_local_toggle(origin + Duration::from_millis(700));
    clock.sync(snapshot(400, true), origin + Duration::from_millis(800));
    assert!(clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_millis(900)),
        Duration::from_millis(550)
    );
}

#[test]
fn manual_stopwatch_reset_deadline_is_not_extended_by_a_later_toggle() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(5_000, true), origin);
    clock.apply_local_reset(origin);
    clock.apply_local_toggle(origin + Duration::from_millis(2_900));

    clock.sync(snapshot(5_200, false), origin + Duration::from_secs(3));

    assert!(!clock.running());
    assert_eq!(
        clock.elapsed_at(origin + Duration::from_secs(30)),
        Duration::from_millis(5_200)
    );
}

#[test]
fn manual_stopwatch_later_toggle_recovers_after_an_unacknowledged_command() {
    let origin = Instant::now();
    let mut clock = ManualStopwatchClock::default();
    clock.sync(snapshot(1_000, false), origin);
    clock.apply_local_toggle(origin);

    // The start command failed: Core remains paused through the deadline.
    clock.sync(snapshot(1_100, false), origin + Duration::from_secs(3));
    assert!(!clock.running());

    let retry_at = origin + Duration::from_millis(3_100);
    clock.apply_local_toggle(retry_at);
    assert!(clock.running());
    clock.sync(snapshot(1_200, true), origin + Duration::from_millis(3_200));
    assert!(clock.running());
}
