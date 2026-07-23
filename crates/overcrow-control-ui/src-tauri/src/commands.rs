use overcrow_control::{ControlCommandService, ControlLogSnapshot, ControlSnapshot};
use tauri::{AppHandle, State};

pub type CommandState = ControlCommandService;

#[tauri::command]
pub fn get_control_state(
    app: AppHandle,
    state: State<'_, CommandState>,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.get_control_state())
}

#[tauri::command]
pub fn refresh_games(
    app: AppHandle,
    state: State<'_, CommandState>,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.refresh_games())
}

#[tauri::command]
pub fn set_game_selected(
    app: AppHandle,
    state: State<'_, CommandState>,
    app_id: u32,
    selected: bool,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.set_game_selected(app_id, selected))
}

#[tauri::command]
pub fn remove_manual_game(
    app: AppHandle,
    state: State<'_, CommandState>,
    id: String,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.remove_manual_game(&id))
}

#[tauri::command]
pub fn pick_manual_game(
    app: AppHandle,
    state: State<'_, CommandState>,
) -> Result<ControlSnapshot, String> {
    sync_tray(&app, state.pick_manual_game())
}

#[tauri::command]
pub fn set_overcrow_enabled(
    app: AppHandle,
    state: State<'_, CommandState>,
    enabled: bool,
) -> Result<ControlSnapshot, String> {
    let result = sync_tray(&app, state.set_overcrow_enabled(enabled));
    if result
        .as_ref()
        .is_ok_and(|snapshot| snapshot.operations.lifecycle)
    {
        crate::tray::ensure_lifecycle_monitor(&app);
    }
    result
}

#[tauri::command]
pub fn get_recent_logs(state: State<'_, CommandState>) -> Result<ControlLogSnapshot, String> {
    state.get_recent_logs()
}

fn sync_tray(
    app: &AppHandle,
    result: Result<ControlSnapshot, String>,
) -> Result<ControlSnapshot, String> {
    if let Ok(snapshot) = &result {
        crate::tray::sync_snapshot(app, snapshot);
    }
    result
}
