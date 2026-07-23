// The test harness skips Tauri's generated context, so its command entrypoints
// are intentionally unreferenced in test builds.
#[cfg_attr(test, allow(dead_code))]
mod commands;

#[cfg(not(test))]
use commands::CommandState;
#[cfg(not(test))]
use overcrow_control::run_settings_diagnostic_request;

#[cfg(not(test))]
fn main() {
    if let Some(status) = run_settings_diagnostic_request() {
        std::process::exit(status);
    }

    let result = tauri::Builder::default()
        .manage(CommandState::production())
        .invoke_handler(tauri::generate_handler![
            commands::get_control_state,
            commands::refresh_games,
            commands::set_game_selected,
            commands::remove_manual_game,
            commands::pick_manual_game,
            commands::set_overcrow_enabled,
            commands::get_recent_logs,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        eprintln!("OverCrow Control Center failed: {error}");
        std::process::exit(1);
    }
}
