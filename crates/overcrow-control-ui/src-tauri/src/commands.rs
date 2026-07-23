use overcrow_control::{ControlCommandService, ControlLogSnapshot, ControlSnapshot};
use tauri::State;

pub type CommandState = ControlCommandService;

#[tauri::command]
pub fn get_control_state(state: State<'_, CommandState>) -> Result<ControlSnapshot, String> {
    state.get_control_state()
}

#[tauri::command]
pub fn refresh_games(state: State<'_, CommandState>) -> Result<ControlSnapshot, String> {
    state.refresh_games()
}

#[tauri::command]
pub fn set_game_selected(
    state: State<'_, CommandState>,
    app_id: u32,
    selected: bool,
) -> Result<ControlSnapshot, String> {
    state.set_game_selected(app_id, selected)
}

#[tauri::command]
pub fn remove_manual_game(
    state: State<'_, CommandState>,
    id: String,
) -> Result<ControlSnapshot, String> {
    state.remove_manual_game(&id)
}

#[tauri::command]
pub fn pick_manual_game(state: State<'_, CommandState>) -> Result<ControlSnapshot, String> {
    state.pick_manual_game()
}

#[tauri::command]
pub fn set_overcrow_enabled(
    state: State<'_, CommandState>,
    enabled: bool,
) -> Result<ControlSnapshot, String> {
    state.set_overcrow_enabled(enabled)
}

#[tauri::command]
pub fn get_recent_logs(state: State<'_, CommandState>) -> Result<ControlLogSnapshot, String> {
    state.get_recent_logs()
}
