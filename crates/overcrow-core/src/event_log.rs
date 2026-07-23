use std::time::{Duration, Instant};

use overcrow_logging::EventLogger;
use overcrow_protocol::{CoreSnapshot, OverlayMode, Rect, VersionedCoreSnapshot};
use tokio::sync::watch;

const GEOMETRY_LOG_INTERVAL: Duration = Duration::from_secs(1);
const MAX_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
struct LoggedGame {
    pid: Option<u32>,
    steam_app_id: Option<u32>,
    app_id: Option<String>,
    backend: String,
    rect: Rect,
}

impl LoggedGame {
    fn from_snapshot(snapshot: &CoreSnapshot) -> Option<Self> {
        let game = snapshot.active_game.as_ref()?;
        Some(Self {
            pid: game.pid,
            steam_app_id: game.steam_app_id,
            app_id: game.app_id.as_deref().map(bounded_identifier),
            backend: bounded_text(&game.backend),
            rect: game.rect.clone(),
        })
    }

    fn same_identity(&self, other: &Self) -> bool {
        self.pid == other.pid
            && self.steam_app_id == other.steam_app_id
            && self.app_id == other.app_id
            && self.backend == other.backend
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CoreEvent {
    GameDetected {
        pid: Option<u32>,
        steam_app_id: Option<u32>,
        app_id: Option<String>,
        backend: String,
        rect: Rect,
    },
    GameChanged {
        pid: Option<u32>,
        steam_app_id: Option<u32>,
        app_id: Option<String>,
        backend: String,
        rect: Rect,
    },
    GeometryChanged {
        rect: Rect,
        suppressed: u64,
    },
    GameCleared,
    OverlayModeChanged(OverlayMode),
}

impl CoreEvent {
    fn game_detected(game: &LoggedGame) -> Self {
        Self::GameDetected {
            pid: game.pid,
            steam_app_id: game.steam_app_id,
            app_id: game.app_id.clone(),
            backend: game.backend.clone(),
            rect: game.rect.clone(),
        }
    }

    fn game_changed(game: &LoggedGame) -> Self {
        Self::GameChanged {
            pid: game.pid,
            steam_app_id: game.steam_app_id,
            app_id: game.app_id.clone(),
            backend: game.backend.clone(),
            rect: game.rect.clone(),
        }
    }

    fn emit(&self, logger: &EventLogger) {
        match self {
            Self::GameDetected {
                pid,
                steam_app_id,
                app_id,
                backend,
                rect,
            }
            | Self::GameChanged {
                pid,
                steam_app_id,
                app_id,
                backend,
                rect,
            } => {
                let name = if matches!(self, Self::GameDetected { .. }) {
                    "game_detected"
                } else {
                    "game_changed"
                };
                logger.info(
                    name,
                    format_args!(
                        "pid={pid:?} steam_app_id={steam_app_id:?} app_id={app_id:?} backend={backend:?} rect={},{},{},{}",
                        rect.x, rect.y, rect.width, rect.height
                    ),
                );
            }
            Self::GeometryChanged { rect, suppressed } => logger.info(
                "game_geometry_changed",
                format_args!(
                    "rect={},{},{},{} suppressed={suppressed}",
                    rect.x, rect.y, rect.width, rect.height
                ),
            ),
            Self::GameCleared => logger.info("game_cleared", format_args!("")),
            Self::OverlayModeChanged(mode) => {
                logger.info("overlay_mode_changed", format_args!("mode={mode:?}"))
            }
        }
    }
}

#[derive(Debug, Default)]
struct CoreEventTracker {
    game: Option<LoggedGame>,
    mode: Option<OverlayMode>,
    next_geometry_log: Option<Instant>,
    pending_geometry: Option<(Rect, u64)>,
}

impl CoreEventTracker {
    fn observe(&mut self, snapshot: &CoreSnapshot, now: Instant) -> Vec<CoreEvent> {
        let current = LoggedGame::from_snapshot(snapshot);
        let mut events = Vec::new();
        match (&self.game, &current) {
            (None, Some(game)) => {
                events.push(CoreEvent::game_detected(game));
                self.reset_geometry_limit();
            }
            (Some(_), None) => {
                self.flush_pending_geometry(&mut events);
                events.push(CoreEvent::GameCleared);
                self.reset_geometry_limit();
            }
            (Some(previous), Some(game)) if !previous.same_identity(game) => {
                self.flush_pending_geometry(&mut events);
                events.push(CoreEvent::game_changed(game));
                self.reset_geometry_limit();
            }
            (Some(previous), Some(game)) if previous.rect != game.rect => {
                self.observe_geometry(game.rect.clone(), now, &mut events);
            }
            _ => self.flush_due_geometry(now, &mut events),
        }
        self.game = current;

        if self.mode != Some(snapshot.overlay_mode) {
            self.mode = Some(snapshot.overlay_mode);
            events.push(CoreEvent::OverlayModeChanged(snapshot.overlay_mode));
        }
        events
    }

    fn observe_geometry(&mut self, rect: Rect, now: Instant, events: &mut Vec<CoreEvent>) {
        if self
            .next_geometry_log
            .is_none_or(|deadline| now >= deadline)
        {
            let suppressed = self
                .pending_geometry
                .take()
                .map_or(0, |(_, suppressed)| suppressed);
            events.push(CoreEvent::GeometryChanged { rect, suppressed });
            self.next_geometry_log = now.checked_add(GEOMETRY_LOG_INTERVAL);
            return;
        }
        let suppressed = self
            .pending_geometry
            .as_ref()
            .map_or(1, |(_, count)| count.saturating_add(1));
        self.pending_geometry = Some((rect, suppressed));
    }

    fn flush_due_geometry(&mut self, now: Instant, events: &mut Vec<CoreEvent>) {
        if self
            .next_geometry_log
            .is_some_and(|deadline| now >= deadline)
        {
            self.flush_pending_geometry(events);
            if events
                .last()
                .is_some_and(|event| matches!(event, CoreEvent::GeometryChanged { .. }))
            {
                self.next_geometry_log = now.checked_add(GEOMETRY_LOG_INTERVAL);
            }
        }
    }

    fn flush_pending_geometry(&mut self, events: &mut Vec<CoreEvent>) {
        if let Some((rect, suppressed)) = self.pending_geometry.take() {
            events.push(CoreEvent::GeometryChanged { rect, suppressed });
        }
    }

    fn reset_geometry_limit(&mut self) {
        self.next_geometry_log = None;
        self.pending_geometry = None;
    }
}

pub async fn run_core_event_logging(
    mut receiver: watch::Receiver<VersionedCoreSnapshot>,
    logger: EventLogger,
) {
    let mut tracker = CoreEventTracker::default();
    loop {
        let events = {
            let current = receiver.borrow_and_update();
            tracker.observe(&current.snapshot, Instant::now())
        };
        for event in events {
            event.emit(&logger);
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}

fn bounded_text(value: &str) -> String {
    let mut end = value.len().min(MAX_ID_BYTES);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn bounded_identifier(value: &str) -> String {
    if value.contains(['/', '\\']) {
        "<redacted-path>".to_owned()
    } else {
        bounded_text(value)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use overcrow_protocol::{CoreSnapshot, GameWindow, OverlayMode, Rect};

    use super::{CoreEvent, CoreEventTracker};

    fn snapshot(backend: &str, rect: Rect) -> CoreSnapshot {
        CoreSnapshot {
            active_game: Some(GameWindow {
                pid: Some(42),
                steam_app_id: Some(1_623_730),
                app_id: Some("steam_app_1623730".to_owned()),
                title: "/home/player/private game title".to_owned(),
                rect,
                scale: 1.25,
                backend: backend.to_owned(),
            }),
            overlay_mode: OverlayMode::Passive,
            ..CoreSnapshot::default()
        }
    }

    fn rect(x: i32, width: u32) -> Rect {
        Rect {
            x,
            y: 36,
            width,
            height: 1080,
        }
    }

    #[test]
    fn common_events_cover_wayland_and_x11_snapshots() {
        for backend in ["wayland", "x11"] {
            let started = Instant::now();
            let mut tracker = CoreEventTracker::default();
            let initial = snapshot(backend, rect(0, 1920));

            assert_eq!(
                tracker.observe(&initial, started),
                vec![
                    CoreEvent::GameDetected {
                        pid: Some(42),
                        steam_app_id: Some(1_623_730),
                        app_id: Some("steam_app_1623730".to_owned()),
                        backend: backend.to_owned(),
                        rect: rect(0, 1920),
                    },
                    CoreEvent::OverlayModeChanged(OverlayMode::Passive),
                ]
            );

            let mut interactive = initial.clone();
            interactive.overlay_mode = OverlayMode::Interactive;
            assert_eq!(
                tracker.observe(&interactive, started + Duration::from_millis(10)),
                vec![CoreEvent::OverlayModeChanged(OverlayMode::Interactive)]
            );

            assert_eq!(
                tracker.observe(&CoreSnapshot::default(), started + Duration::from_secs(1)),
                vec![
                    CoreEvent::GameCleared,
                    CoreEvent::OverlayModeChanged(OverlayMode::Passive),
                ]
            );
        }
    }

    #[test]
    fn geometry_is_limited_to_one_event_per_second_and_reports_suppression() {
        let started = Instant::now();
        let mut tracker = CoreEventTracker::default();
        tracker.observe(&snapshot("wayland", rect(0, 1920)), started);

        assert_eq!(
            tracker.observe(
                &snapshot("wayland", rect(1, 1919)),
                started + Duration::from_millis(1),
            ),
            vec![CoreEvent::GeometryChanged {
                rect: rect(1, 1919),
                suppressed: 0,
            }]
        );
        assert!(
            tracker
                .observe(
                    &snapshot("wayland", rect(2, 1918)),
                    started + Duration::from_millis(33),
                )
                .is_empty()
        );
        assert!(
            tracker
                .observe(
                    &snapshot("wayland", rect(3, 1917)),
                    started + Duration::from_millis(66),
                )
                .is_empty()
        );

        assert_eq!(
            tracker.observe(
                &snapshot("wayland", rect(3, 1917)),
                started + Duration::from_millis(1001),
            ),
            vec![CoreEvent::GeometryChanged {
                rect: rect(3, 1917),
                suppressed: 2,
            }]
        );
    }

    #[test]
    fn tracker_does_not_retain_window_titles_or_unbounded_identifiers() {
        let mut tracker = CoreEventTracker::default();
        let mut private = snapshot("wayland", rect(0, 1920));
        private.active_game.as_mut().expect("active game").app_id =
            Some(format!("/home/player/{}", "a".repeat(8_192)));

        let events = tracker.observe(&private, Instant::now());
        let rendered = format!("{events:?}");

        assert!(!rendered.contains("/home/player"));
        assert!(rendered.len() < 1_024);
    }
}
