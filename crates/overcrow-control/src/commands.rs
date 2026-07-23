use std::{
    io,
    sync::{Mutex, MutexGuard},
};

use crate::model::is_stable_manual_game_id;
use crate::{ControlController, ControlLogSnapshot, ControlSnapshot, MAX_CONTROL_LOG_LINES};

type LogReader = fn(usize) -> io::Result<Vec<String>>;

/// Serializes access to the native Control Center authority.
pub struct ControlCommandService {
    controller: Mutex<ControlController>,
    log_reader: LogReader,
}

impl ControlCommandService {
    pub fn production() -> Self {
        Self::new(ControlController::production())
    }

    pub fn new(controller: ControlController) -> Self {
        Self {
            controller: Mutex::new(controller),
            log_reader: overcrow_logging::read_recent_logs,
        }
    }

    #[cfg(test)]
    fn new_with_log_reader(controller: ControlController, log_reader: LogReader) -> Self {
        Self {
            controller: Mutex::new(controller),
            log_reader,
        }
    }

    pub fn get_control_state(&self) -> Result<ControlSnapshot, String> {
        let mut controller = self.lock()?;
        controller.poll_pending();
        Ok(controller.snapshot())
    }

    pub fn refresh_games(&self) -> Result<ControlSnapshot, String> {
        let mut controller = self.lock()?;
        controller.start_refresh();
        Ok(controller.snapshot())
    }

    pub fn set_game_selected(
        &self,
        app_id: u32,
        selected: bool,
    ) -> Result<ControlSnapshot, String> {
        if app_id == 0 {
            return Err("invalid_app_id".to_owned());
        }
        let mut controller = self.lock()?;
        if !controller
            .model()
            .games
            .iter()
            .any(|game| game.app_id == app_id)
        {
            return Err("unknown_app_id".to_owned());
        }
        controller.set_steam_selected(app_id, selected);
        Ok(controller.snapshot())
    }

    pub fn remove_manual_game(&self, id: &str) -> Result<ControlSnapshot, String> {
        if !is_stable_manual_game_id(id) {
            return Err("invalid_manual_game_id".to_owned());
        }
        let mut controller = self.lock()?;
        controller.remove_manual_game(id);
        Ok(controller.snapshot())
    }

    pub fn pick_manual_game(&self) -> Result<ControlSnapshot, String> {
        let mut controller = self.lock()?;
        controller.start_native_picker();
        Ok(controller.snapshot())
    }

    pub fn set_overcrow_enabled(&self, enabled: bool) -> Result<ControlSnapshot, String> {
        let mut controller = self.lock()?;
        if enabled && !controller.snapshot().compatibility.activation_allowed {
            return Err("unsupported_environment".to_owned());
        }
        if !controller.request_master_toggle(enabled) {
            return Err("request_rejected".to_owned());
        }
        Ok(controller.snapshot())
    }

    pub fn get_recent_logs(&self) -> Result<ControlLogSnapshot, String> {
        let lines = (self.log_reader)(MAX_CONTROL_LOG_LINES + 1)
            .map_err(|_| "logs_unavailable".to_owned())?;
        ControlLogSnapshot::from_recent_lines(lines).ok_or_else(|| "logs_invalid".to_owned())
    }

    fn lock(&self) -> Result<MutexGuard<'_, ControlController>, String> {
        self.controller
            .lock()
            .map_err(|_| "state_unavailable".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::{io, path::PathBuf};

    use overcrow_config::{LifecycleSettings, SettingsLoad};

    use crate::{
        ControlController, ControlModel, DiagnosticInput, DiscoveryReport, NativePathValidator,
        SelectionStore, SteamGame,
    };

    use super::{ControlCommandService, LogReader};

    struct MemoryStore;

    impl SelectionStore for MemoryStore {
        fn save(&self, _settings: &LifecycleSettings) -> io::Result<()> {
            Ok(())
        }
    }

    fn commands(desktop: &str) -> ControlCommandService {
        let model = ControlModel::new(
            SettingsLoad {
                settings: LifecycleSettings::default(),
                warning: None,
            },
            DiscoveryReport {
                games: vec![SteamGame {
                    app_id: 4242,
                    name: "Example Game".to_owned(),
                    install_dir: PathBuf::from("/games/example"),
                    icon: None,
                }],
                warnings: Vec::new(),
            },
            NativePathValidator,
        );
        ControlCommandService::new(ControlController::new_with_diagnostic_input(
            model,
            MemoryStore,
            DiagnosticInput {
                session_type: Some("wayland".to_owned()),
                current_desktop: Some(desktop.to_owned()),
                desktop_session: Some(desktop.to_owned()),
                ..DiagnosticInput::default()
            },
        ))
    }

    fn commands_with_log_reader(desktop: &str, log_reader: LogReader) -> ControlCommandService {
        let commands = commands(desktop);
        ControlCommandService::new_with_log_reader(
            commands.controller.into_inner().expect("controller"),
            log_reader,
        )
    }

    fn fixture_logs(limit: usize) -> io::Result<Vec<String>> {
        assert_eq!(limit, crate::MAX_CONTROL_LOG_LINES + 1);
        Ok(vec![
            "2026-07-23T10:00:00.000Z INFO core started".to_owned(),
            "2026-07-23T10:00:01.000Z WARN overlay frame_late".to_owned(),
        ])
    }

    fn unavailable_logs(_limit: usize) -> io::Result<Vec<String>> {
        Err(io::Error::new(io::ErrorKind::NotFound, "fixture"))
    }

    #[test]
    fn command_service_validates_game_selection_and_identifiers() {
        let commands = commands("Hyprland");
        assert!(!commands.get_control_state().expect("state").games[0].selected);
        assert!(
            commands
                .set_game_selected(4242, true)
                .expect("select")
                .games[0]
                .selected
        );
        assert_eq!(
            commands.set_game_selected(0, true).unwrap_err(),
            "invalid_app_id"
        );
        assert_eq!(
            commands.set_game_selected(99, true).unwrap_err(),
            "unknown_app_id"
        );
        assert_eq!(
            commands.remove_manual_game("../bad").unwrap_err(),
            "invalid_manual_game_id"
        );
        assert!(
            commands
                .remove_manual_game("local.0123456789abcdef0123456789abcdef")
                .is_ok()
        );
        assert_eq!(
            commands
                .remove_manual_game("local.0123456789ABCDEF0123456789ABCDEF")
                .unwrap_err(),
            "invalid_manual_game_id"
        );
    }

    #[test]
    fn unsupported_sessions_cannot_request_activation() {
        let commands = commands("GNOME");
        assert_eq!(
            commands.set_overcrow_enabled(true).unwrap_err(),
            "unsupported_environment"
        );
    }

    #[test]
    fn recent_logs_use_the_fixed_bounded_reader() {
        let commands = commands_with_log_reader("Hyprland", fixture_logs);
        let snapshot = commands.get_recent_logs().expect("recent logs");
        assert_eq!(snapshot.schema_version, crate::CONTROL_LOG_SCHEMA_VERSION);
        assert_eq!(snapshot.lines.len(), 2);
        assert!(!snapshot.truncated);
    }

    #[test]
    fn recent_log_reader_failures_use_one_stable_code() {
        let commands = commands_with_log_reader("Hyprland", unavailable_logs);
        assert_eq!(commands.get_recent_logs().unwrap_err(), "logs_unavailable");
    }
}
