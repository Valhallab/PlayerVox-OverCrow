use std::time::{Duration, Instant};

const WARFRAME_TICK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct MarketDeadline {
    revision: u64,
    wall_secs: u64,
    monotonic: Instant,
    fired: bool,
}

#[derive(Default)]
pub struct OverlayScheduler {
    warframe_tick_at: Option<Instant>,
    market_refresh: Option<MarketDeadline>,
    copy_flash_at: Option<Instant>,
    last_observed_wall_secs: Option<u64>,
}

impl OverlayScheduler {
    pub fn take_warframe_tick(&mut self, enabled: bool, now: Instant) -> bool {
        if !enabled {
            self.warframe_tick_at = None;
            return false;
        }
        let due = self.warframe_tick_at.is_none_or(|deadline| now >= deadline);
        if due {
            self.warframe_tick_at = now.checked_add(WARFRAME_TICK_INTERVAL);
        }
        due
    }

    #[allow(clippy::too_many_arguments)]
    pub fn take_market_refresh(
        &mut self,
        enabled: bool,
        has_selection: bool,
        revision: u64,
        refresh_at_wall_secs: u64,
        now: Instant,
        wall_secs: u64,
    ) -> bool {
        self.last_observed_wall_secs = Some(wall_secs);
        if !enabled || !has_selection || refresh_at_wall_secs == u64::MAX {
            self.market_refresh = None;
            return false;
        }

        let changed = self.market_refresh.is_none_or(|deadline| {
            deadline.revision != revision || deadline.wall_secs != refresh_at_wall_secs
        });
        if changed {
            let delay = Duration::from_secs(refresh_at_wall_secs.saturating_sub(wall_secs));
            self.market_refresh = Some(MarketDeadline {
                revision,
                wall_secs: refresh_at_wall_secs,
                monotonic: now.checked_add(delay).unwrap_or(now),
                fired: false,
            });
        }

        let deadline = self
            .market_refresh
            .as_mut()
            .expect("active market work has a deadline");
        if deadline.fired || (now < deadline.monotonic && wall_secs < deadline.wall_secs) {
            return false;
        }
        deadline.fired = true;
        true
    }

    pub fn set_copy_flash_deadline(&mut self, deadline: Option<Instant>) {
        self.copy_flash_at = deadline;
    }

    pub fn take_copy_flash_expired(&mut self, now: Instant) -> bool {
        if self.copy_flash_at.is_none_or(|deadline| now < deadline) {
            return false;
        }
        self.copy_flash_at = None;
        true
    }

    pub fn next_repaint_after(&self, now: Instant) -> Option<Duration> {
        let market = self
            .market_refresh
            .filter(|deadline| !deadline.fired)
            .map(|deadline| deadline.monotonic);
        [self.warframe_tick_at, market, self.copy_flash_at]
            .into_iter()
            .flatten()
            .min()
            .map(|deadline| deadline.saturating_duration_since(now))
    }
}
