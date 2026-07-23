// The test harness skips Tauri's generated context, so its command entrypoints
// are intentionally unreferenced in test builds.
#[cfg_attr(test, allow(dead_code))]
mod commands;
#[cfg_attr(test, allow(dead_code))]
mod single_instance;
#[cfg_attr(test, allow(dead_code))]
mod tray;
#[cfg(test)]
mod tray_tests;

#[cfg(not(test))]
use commands::CommandState;
#[cfg(not(test))]
use overcrow_control::run_settings_diagnostic_request;
#[cfg(not(test))]
use tauri::WindowEvent;

#[cfg(not(test))]
fn main() {
    if let Some(status) = run_settings_diagnostic_request() {
        std::process::exit(status);
    }

    let builder = tauri::Builder::default()
        .manage(CommandState::production())
        .setup(|app| {
            if let Err(error) = tray::install(app) {
                abort_startup(app, &format!("could not install the system tray: {error}"));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    eprintln!("OverCrow could not hide the Control Center: {error}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_control_state,
            commands::refresh_games,
            commands::set_game_selected,
            commands::remove_manual_game,
            commands::pick_manual_game,
            commands::set_overcrow_enabled,
            commands::get_recent_logs,
        ]);

    let mut app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("OverCrow Control Center failed: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = single_instance::install(&mut app) {
        abort_startup(&mut app, &error);
    }
    app.run(|_, _| {});
}

#[cfg(not(test))]
fn abort_startup(app: &mut tauri::App, error: &str) -> ! {
    eprintln!("OverCrow Control Center failed: {error}");
    app.cleanup_before_exit();
    std::process::exit(1);
}
