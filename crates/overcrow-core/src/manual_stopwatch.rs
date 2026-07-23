use std::time::{Duration, Instant};

use overcrow_protocol::ManualStopwatchSnapshot;

#[derive(Debug, Default)]
pub struct ManualStopwatch {
    accumulated: Duration,
    started_at: Option<Instant>,
}

impl ManualStopwatch {
    pub fn toggle(&mut self, now: Instant) {
        match self.started_at.take() {
            Some(started_at) => {
                self.accumulated = self
                    .accumulated
                    .saturating_add(now.checked_duration_since(started_at).unwrap_or_default());
            }
            None => self.started_at = Some(now),
        }
    }

    pub fn reset(&mut self) {
        self.accumulated = Duration::ZERO;
        self.started_at = None;
    }

    pub fn snapshot_at(&self, now: Instant) -> ManualStopwatchSnapshot {
        let elapsed = self.started_at.map_or(self.accumulated, |started_at| {
            self.accumulated
                .saturating_add(now.checked_duration_since(started_at).unwrap_or_default())
        });
        ManualStopwatchSnapshot {
            elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            running: self.started_at.is_some(),
        }
    }
}
