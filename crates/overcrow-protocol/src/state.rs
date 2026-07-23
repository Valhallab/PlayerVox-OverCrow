use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameWindow {
    pub pid: Option<u32>,
    pub steam_app_id: Option<u32>,
    pub app_id: Option<String>,
    pub title: String,
    pub rect: Rect,
    pub scale: f64,
    pub backend: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverlayMode {
    #[default]
    Passive,
    Interactive,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameTelemetry {
    pub cpu_percent_hundredths: Option<u32>,
    pub resident_bytes: Option<u64>,
    pub cpu_temperature_millicelsius: Option<i64>,
    pub gpu_temperature_millicelsius: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualStopwatchSnapshot {
    pub elapsed_ms: u64,
    pub running: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoreSnapshot {
    pub active_game: Option<GameWindow>,
    pub overlay_mode: OverlayMode,
    #[serde(default)]
    pub session_elapsed_ms: Option<u64>,
    #[serde(default)]
    pub telemetry: Option<GameTelemetry>,
    #[serde(default)]
    pub manual_stopwatch: ManualStopwatchSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VersionedCoreSnapshot {
    pub revision: u64,
    pub snapshot: CoreSnapshot,
}

#[derive(Default)]
pub struct CoreState {
    snapshot: CoreSnapshot,
}

impl CoreState {
    pub fn snapshot(&self) -> &CoreSnapshot {
        &self.snapshot
    }

    pub fn observe_game(&mut self, game: GameWindow) {
        self.snapshot.active_game = Some(game);
    }

    pub fn clear_game(&mut self) {
        self.snapshot.active_game = None;
        self.snapshot.overlay_mode = OverlayMode::Passive;
        self.snapshot.session_elapsed_ms = None;
    }

    pub fn toggle_overlay(&mut self) {
        if self.snapshot.active_game.is_none() {
            self.snapshot.overlay_mode = OverlayMode::Passive;
            return;
        }

        self.snapshot.overlay_mode = match self.snapshot.overlay_mode {
            OverlayMode::Passive => OverlayMode::Interactive,
            OverlayMode::Interactive => OverlayMode::Passive,
        };
    }

    pub fn set_overlay_interactive(&mut self, interactive: bool) {
        self.snapshot.overlay_mode = if interactive && self.snapshot.active_game.is_some() {
            OverlayMode::Interactive
        } else {
            OverlayMode::Passive
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_game() -> GameWindow {
        GameWindow {
            pid: Some(42),
            steam_app_id: Some(620),
            app_id: Some("portal2".to_owned()),
            title: "Portal 2".to_owned(),
            rect: Rect {
                x: 100,
                y: 200,
                width: 1920,
                height: 1080,
            },
            scale: 1.0,
            backend: "x11".to_owned(),
        }
    }

    #[test]
    fn cannot_be_interactive_without_a_game() {
        let mut state = CoreState::default();
        state.toggle_overlay();
        assert_eq!(state.snapshot().overlay_mode, OverlayMode::Passive);
    }

    #[test]
    fn toggling_with_a_game_becomes_interactive() {
        let mut state = CoreState::default();
        state.observe_game(sample_game());
        state.toggle_overlay();
        assert_eq!(state.snapshot().overlay_mode, OverlayMode::Interactive);
    }

    #[test]
    fn clearing_game_forces_passthrough() {
        let mut state = CoreState::default();
        state.observe_game(sample_game());
        state.toggle_overlay();
        state.clear_game();
        assert_eq!(state.snapshot().overlay_mode, OverlayMode::Passive);
    }

    #[test]
    fn json_round_trip_preserves_a_snapshot() {
        let mut state = CoreState::default();
        state.observe_game(sample_game());
        state.toggle_overlay();

        let json = serde_json::to_string(state.snapshot()).expect("snapshot serializes");
        let decoded: CoreSnapshot = serde_json::from_str(&json).expect("snapshot deserializes");

        assert_eq!(decoded, *state.snapshot());
    }

    #[test]
    fn setting_interactive_without_a_game_stays_passive() {
        let mut state = CoreState::default();
        state.set_overlay_interactive(true);
        assert_eq!(state.snapshot().overlay_mode, OverlayMode::Passive);
    }

    #[test]
    fn setting_interactive_with_a_game_becomes_interactive() {
        let mut state = CoreState::default();
        state.observe_game(sample_game());
        state.set_overlay_interactive(true);
        assert_eq!(state.snapshot().overlay_mode, OverlayMode::Interactive);
    }

    #[test]
    fn missing_session_elapsed_is_backward_compatible() {
        let decoded: CoreSnapshot =
            serde_json::from_str(r#"{"active_game":null,"overlay_mode":"Passive"}"#)
                .expect("legacy snapshot");

        assert_eq!(decoded.session_elapsed_ms, None);
    }

    #[test]
    fn old_snapshots_default_new_optional_runtime_data() {
        let snapshot: CoreSnapshot =
            serde_json::from_str(r#"{"active_game":null,"overlay_mode":"Passive"}"#).unwrap();

        assert_eq!(snapshot.telemetry, None);
        assert_eq!(
            snapshot.manual_stopwatch,
            ManualStopwatchSnapshot::default()
        );
    }

    #[test]
    fn runtime_values_round_trip_without_floats() {
        let telemetry = GameTelemetry {
            cpu_percent_hundredths: Some(12_345),
            resident_bytes: Some(4_294_967_296),
            cpu_temperature_millicelsius: Some(62_000),
            gpu_temperature_millicelsius: None,
        };

        assert_eq!(
            serde_json::from_str::<GameTelemetry>(&serde_json::to_string(&telemetry).unwrap(),)
                .unwrap(),
            telemetry
        );
    }

    #[test]
    fn session_elapsed_round_trips() {
        let snapshot = CoreSnapshot {
            session_elapsed_ms: Some(1_234),
            ..CoreSnapshot::default()
        };
        let json = serde_json::to_string(&snapshot).expect("serialize snapshot");

        assert_eq!(
            serde_json::from_str::<CoreSnapshot>(&json).expect("decode snapshot"),
            snapshot
        );
    }

    #[test]
    fn versioned_snapshot_round_trips_with_its_revision() {
        let event = VersionedCoreSnapshot {
            revision: 7,
            snapshot: CoreSnapshot {
                overlay_mode: OverlayMode::Interactive,
                ..CoreSnapshot::default()
            },
        };
        let json = serde_json::to_string(&event).expect("serialize event");
        assert_eq!(
            serde_json::from_str::<VersionedCoreSnapshot>(&json).expect("decode event"),
            event
        );
    }
}
