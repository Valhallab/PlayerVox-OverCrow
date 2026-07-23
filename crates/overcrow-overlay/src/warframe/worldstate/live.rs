//! Keep continuous worldstate timers live between polls.
//!
//! Open-world phases do not expire: when one ends the next begins. Between
//! worldstate polls we advance phase state from fixed cycle math so the UI
//! never shows `expired` for Cetus/Vallis/Cambion/Zariman/daily reset.

use super::parse::{CETUS_DAY_SECS, CETUS_NIGHT_SECS, vallis_at, zariman_at};
use crate::warframe::model::{CycleStatus, WorldstateSnapshot};

/// Advance continuous timers to `now_secs`.
///
/// Returns `true` when a bounty-backed phase rolled so a worldstate re-fetch
/// is useful. Vallis/daily reset are pure math and never force a fetch.
/// Callers schedule this at the one-second presentation boundary; the function
/// deliberately does not perform frame-rate throttling or request repaints.
pub fn advance_live_timers(snapshot: &mut WorldstateSnapshot, now_secs: u64) -> bool {
    if snapshot.cycles.is_empty() && snapshot.daily_reset_at_secs.is_none() {
        return false;
    }

    let mut needs_refresh = false;
    let now_ms = i64::try_from(now_secs.saturating_mul(1_000)).unwrap_or(i64::MAX);

    for cycle in &mut snapshot.cycles {
        let id = cycle.id.as_str();
        if id == "cetus" {
            needs_refresh |= advance_cetus_like(cycle, now_secs, "day", "night");
        } else if id == "cambion" {
            needs_refresh |= advance_cetus_like(cycle, now_secs, "fass", "vome");
        } else if id == "vallis" {
            *cycle = vallis_at(now_ms);
        } else if id == "zariman" && cycle.expires_at_secs <= now_secs {
            *cycle = zariman_at(now_ms);
            needs_refresh = true;
        }
    }

    if let Some(reset) = snapshot.daily_reset_at_secs.as_mut() {
        while *reset <= now_secs {
            *reset = reset.saturating_add(86_400);
        }
    }

    needs_refresh
}

/// Walk day/night (or fass/vome) until `expires_at` is in the future.
/// Returns whether any phase boundary was crossed.
fn advance_cetus_like(cycle: &mut CycleStatus, now_secs: u64, day: &str, night: &str) -> bool {
    if cycle.expires_at_secs > now_secs {
        return false;
    }

    let mut is_day = cycle.state.as_deref() == Some(day) || cycle.state.is_none();
    let mut expires = cycle.expires_at_secs;
    // At most a few steps even after a long sleep (day+night ≈ 2.5h).
    for _ in 0..64 {
        if expires > now_secs {
            break;
        }
        if is_day {
            is_day = false;
            expires = expires.saturating_add(CETUS_NIGHT_SECS);
        } else {
            is_day = true;
            expires = expires.saturating_add(CETUS_DAY_SECS);
        }
    }

    cycle.state = Some(if is_day {
        day.to_owned()
    } else {
        night.to_owned()
    });
    cycle.expires_at_secs = expires.max(now_secs.saturating_add(1));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warframe::model::CycleStatus;

    fn snapshot_with(cycles: Vec<CycleStatus>) -> WorldstateSnapshot {
        WorldstateSnapshot {
            cycles,
            daily_reset_at_secs: Some(1_000),
            ..WorldstateSnapshot::default()
        }
    }

    #[test]
    fn cetus_day_rolls_into_night() {
        let mut snap = snapshot_with(vec![CycleStatus {
            id: "cetus".to_owned(),
            label: "Cetus".to_owned(),
            state: Some("day".to_owned()),
            expires_at_secs: 1_000,
        }]);
        assert!(advance_live_timers(&mut snap, 1_000));
        assert_eq!(snap.cycles[0].state.as_deref(), Some("night"));
        assert_eq!(snap.cycles[0].expires_at_secs, 4_000);
    }

    #[test]
    fn cetus_walks_multiple_phases() {
        let mut snap = snapshot_with(vec![CycleStatus {
            id: "cetus".to_owned(),
            label: "Cetus".to_owned(),
            state: Some("day".to_owned()),
            expires_at_secs: 1_000,
        }]);
        advance_live_timers(&mut snap, 4_100);
        assert_eq!(snap.cycles[0].state.as_deref(), Some("day"));
        assert_eq!(snap.cycles[0].expires_at_secs, 10_000);
    }

    #[test]
    fn active_phase_is_left_alone() {
        let mut snap = snapshot_with(vec![CycleStatus {
            id: "cetus".to_owned(),
            label: "Cetus".to_owned(),
            state: Some("day".to_owned()),
            expires_at_secs: 10_000,
        }]);
        snap.daily_reset_at_secs = Some(100_000);
        assert!(!advance_live_timers(&mut snap, 1_000));
        assert_eq!(snap.cycles[0].expires_at_secs, 10_000);
    }

    #[test]
    fn daily_reset_rolls_forward() {
        let mut snap = snapshot_with(vec![]);
        snap.daily_reset_at_secs = Some(1_000);
        assert!(!advance_live_timers(&mut snap, 1_000));
        assert_eq!(snap.daily_reset_at_secs, Some(87_400));
    }

    #[test]
    fn vallis_stale_row_is_recomputed() {
        let mut snap = snapshot_with(vec![CycleStatus {
            id: "vallis".to_owned(),
            label: "Vallis".to_owned(),
            state: Some("warm".to_owned()),
            expires_at_secs: 1,
        }]);
        let now = 1_784_327_796_u64;
        advance_live_timers(&mut snap, now);
        assert!(snap.cycles[0].expires_at_secs > now);
    }
}
