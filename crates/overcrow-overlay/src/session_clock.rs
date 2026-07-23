use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
struct SessionAnchor {
    elapsed: Duration,
    received_at: Instant,
}

#[derive(Debug, Default)]
pub struct SessionClock {
    anchor: Option<SessionAnchor>,
}

impl SessionClock {
    pub fn sync(&mut self, elapsed_ms: Option<u64>, received_at: Instant) {
        self.anchor = elapsed_ms.map(|elapsed_ms| SessionAnchor {
            elapsed: Duration::from_millis(elapsed_ms),
            received_at,
        });
    }

    pub fn elapsed_at(&self, now: Instant) -> Option<Duration> {
        let anchor = self.anchor?;
        Some(
            anchor
                .elapsed
                .saturating_add(now.saturating_duration_since(anchor.received_at)),
        )
    }

    pub fn repaint_after(&self, now: Instant) -> Option<Duration> {
        let elapsed = self.elapsed_at(now)?;
        let remainder = elapsed.subsec_millis();
        Some(Duration::from_millis(u64::from(1_000 - remainder)))
    }
}

#[cfg(test)]
mod tests {
    use super::SessionClock;
    use std::time::{Duration, Instant};

    #[test]
    fn interpolates_from_the_latest_core_sample() {
        let now = Instant::now();
        let mut clock = SessionClock::default();
        clock.sync(Some(1_200_250), now);

        assert_eq!(
            clock.elapsed_at(now + Duration::from_millis(750)),
            Some(Duration::from_secs(1_201))
        );
    }

    #[test]
    fn a_new_sample_resynchronizes_the_display() {
        let now = Instant::now();
        let mut clock = SessionClock::default();
        clock.sync(Some(1_000), now);
        clock.sync(Some(5_000), now + Duration::from_secs(2));

        assert_eq!(
            clock.elapsed_at(now + Duration::from_secs(3)),
            Some(Duration::from_secs(6))
        );
    }

    #[test]
    fn unavailable_sample_clears_the_anchor() {
        let now = Instant::now();
        let mut clock = SessionClock::default();
        clock.sync(Some(1_000), now);
        clock.sync(None, now + Duration::from_secs(1));

        assert_eq!(clock.elapsed_at(now + Duration::from_secs(2)), None);
        assert_eq!(clock.repaint_after(now + Duration::from_secs(2)), None);
    }

    #[test]
    fn repaint_targets_the_next_displayed_second() {
        let now = Instant::now();
        let mut clock = SessionClock::default();
        clock.sync(Some(1_200_250), now);

        assert_eq!(clock.repaint_after(now), Some(Duration::from_millis(750)));
        assert_eq!(
            clock.repaint_after(now + Duration::from_millis(750)),
            Some(Duration::from_secs(1))
        );
    }
}
