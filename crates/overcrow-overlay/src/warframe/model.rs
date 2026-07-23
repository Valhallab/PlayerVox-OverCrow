use overcrow_config::FissureEra;

pub const STRING_MAX_CHARS: usize = 96;
pub const FISSURE_LIST_MAX: usize = 96;
pub const CYCLE_LIST_MAX: usize = 16;
pub const INVASION_LIST_MAX: usize = 32;
pub const ACTIVITY_MISSION_MAX: usize = 3;
pub const ERROR_MAX_CHARS: usize = 160;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorldstateSnapshot {
    pub server_time_secs: u64,
    /// Wall-clock timestamp of the most recent successful provider response.
    pub fetched_at_secs: u64,
    /// Wall-clock timestamp of the most recent provider attempt, successful or not.
    pub last_attempt_at_secs: u64,
    pub cycles: Vec<CycleStatus>,
    pub daily_reset_at_secs: Option<u64>,
    pub baro: Option<BaroStatus>,
    pub fissures: Vec<FissureMission>,
    pub sortie: Option<SortieMission>,
    pub archon: Option<ArchonHunt>,
    pub invasions: Vec<InvasionMission>,
    pub error: Option<String>,
}

impl WorldstateSnapshot {
    /// True when this snapshot carries usable data (not only an error shell).
    pub fn has_payload(&self) -> bool {
        self.server_time_secs > 0
            || !self.cycles.is_empty()
            || self.baro.is_some()
            || !self.fissures.is_empty()
            || self.daily_reset_at_secs.is_some()
            || self.sortie.is_some()
            || self.archon.is_some()
            || !self.invasions.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityMission {
    pub mission_type: String,
    pub node: String,
    pub modifier: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SortieMission {
    pub boss: String,
    pub expires_at_secs: u64,
    pub missions: Vec<ActivityMission>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchonHunt {
    pub boss: String,
    pub expires_at_secs: u64,
    pub missions: Vec<ActivityMission>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardLine {
    /// Path tail used for watchlist matching (e.g. `SnipetronVandalBlueprint`).
    pub item_key: String,
    pub label: String,
    pub count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvasionMission {
    pub instance_id: String,
    pub node: String,
    pub attacker_faction: String,
    pub defender_faction: String,
    pub attacker_reward: Option<RewardLine>,
    pub defender_reward: Option<RewardLine>,
    pub count: i64,
    pub goal: i64,
    pub completed: bool,
}

impl InvasionMission {
    /// Progress 0.0..=1.0 toward completion (either side), if goal is usable.
    pub fn progress_ratio(&self) -> Option<f32> {
        if self.goal == 0 {
            return None;
        }
        let goal = self.goal.unsigned_abs() as f32;
        let progress = self.count.unsigned_abs() as f32;
        Some((progress / goal).clamp(0.0, 1.0))
    }

    pub fn progress_percent(&self) -> Option<u8> {
        self.progress_ratio()
            .map(|ratio| (ratio * 100.0).round().clamp(0.0, 100.0) as u8)
    }

    /// Attacker share of a tug-of-war bar (0 = full defender, 1 = full attacker).
    /// DE `Count` is signed: positive favors the attacker, negative the defender.
    pub fn attacker_bar_ratio(&self) -> Option<f32> {
        if self.goal == 0 {
            return None;
        }
        let goal = self.goal.unsigned_abs() as f32;
        let t = (self.count as f32 / goal).clamp(-1.0, 1.0);
        Some(((t + 1.0) * 0.5).clamp(0.0, 1.0))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CycleStatus {
    pub id: String,
    pub label: String,
    pub state: Option<String>,
    pub expires_at_secs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaroStatus {
    pub present: bool,
    pub activation_secs: u64,
    pub expiry_secs: u64,
    pub location: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FissureMission {
    pub era: FissureEra,
    pub mission_type: String,
    pub node: String,
    pub expires_at_secs: u64,
    pub steel_path: bool,
    /// Railjack void storm (from `VoidStorms`), not a star-chart fissure.
    pub is_storm: bool,
}

pub fn bound_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_owned();
    }
    let mut out: String = input.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Format a countdown. With `show_seconds`, includes `ss`; without, rounds up to
/// the next minute when under one minute so the timer never looks empty.
///
/// Examples: `2d 5h 12m 03s`, `5h 12m`, `12m`, `03s`, `1m`, `expired`.
pub fn format_remaining(now_secs: u64, expires_at_secs: u64, show_seconds: bool) -> String {
    if expires_at_secs <= now_secs {
        return "expired".to_owned();
    }
    let remaining = expires_at_secs - now_secs;
    if show_seconds {
        let days = remaining / 86_400;
        let hours = (remaining % 86_400) / 3_600;
        let minutes = (remaining % 3_600) / 60;
        let seconds = remaining % 60;
        if days > 0 {
            format!("{days}d {hours}h {minutes:02}m {seconds:02}s")
        } else if hours > 0 {
            format!("{hours}h {minutes:02}m {seconds:02}s")
        } else if minutes > 0 {
            format!("{minutes}m {seconds:02}s")
        } else {
            format!("{seconds:02}s")
        }
    } else {
        // Ceil to whole minutes so sub-minute remainders still read as `1m`.
        let total_minutes = remaining.div_ceil(60);
        let days = total_minutes / (24 * 60);
        let hours = (total_minutes % (24 * 60)) / 60;
        let minutes = total_minutes % 60;
        if days > 0 {
            format!("{days}d {hours}h {minutes:02}m")
        } else if hours > 0 {
            format!("{hours}h {minutes:02}m")
        } else {
            format!("{minutes}m")
        }
    }
}

#[cfg(test)]
mod remaining_tests {
    use super::format_remaining;

    #[test]
    fn remaining_includes_seconds_when_enabled() {
        assert_eq!(format_remaining(100, 100, true), "expired");
        assert_eq!(format_remaining(100, 145, true), "45s");
        assert_eq!(format_remaining(100, 100 + 125, true), "2m 05s");
        assert_eq!(format_remaining(100, 100 + 3_725, true), "1h 02m 05s");
        assert_eq!(
            format_remaining(100, 100 + 86_400 + 3_725, true),
            "1d 1h 02m 05s"
        );
    }

    #[test]
    fn remaining_omits_seconds_when_disabled() {
        assert_eq!(format_remaining(100, 145, false), "1m");
        assert_eq!(format_remaining(100, 100 + 125, false), "3m");
        assert_eq!(format_remaining(100, 100 + 3_725, false), "1h 03m");
        assert_eq!(
            format_remaining(100, 100 + 86_400 + 3_725, false),
            "1d 1h 03m"
        );
    }
}
