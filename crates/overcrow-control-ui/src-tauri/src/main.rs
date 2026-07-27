use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

#[cfg(not(test))]
use std::{env, os::unix::process::CommandExt, process::Command};

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
use commands::{CommandState, SupportReportState};
#[cfg(not(test))]
use overcrow_control::run_settings_diagnostic_request;
#[cfg(not(test))]
use tauri::WindowEvent;

const NVIDIA_PCI_VENDOR: &[u8] = b"0x10de\n";
#[cfg(not(test))]
const PCI_DEVICES_PATH: &str = "/sys/bus/pci/devices";
const MAX_PCI_DEVICES: usize = 4_096;
const MAX_PCI_VENDOR_BYTES: u64 = 16;
#[cfg(not(test))]
const WEBKIT_DMABUF_VARIABLE: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

fn should_restart_with_safe_webkit_renderer(
    wayland_session: bool,
    nvidia_device_present: bool,
    renderer_was_configured: bool,
) -> bool {
    wayland_session && nvidia_device_present && !renderer_was_configured
}

fn nvidia_pci_device_present(root: &Path) -> bool {
    let Ok(devices) = fs::read_dir(root) else {
        return false;
    };
    devices
        .take(MAX_PCI_DEVICES)
        .filter_map(Result::ok)
        .any(|device| vendor_file_is_nvidia(&device.path().join("vendor")))
}

fn vendor_file_is_nvidia(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut vendor = Vec::with_capacity(NVIDIA_PCI_VENDOR.len());
    if file
        .take(MAX_PCI_VENDOR_BYTES + 1)
        .read_to_end(&mut vendor)
        .is_err()
        || vendor.len() as u64 > MAX_PCI_VENDOR_BYTES
    {
        return false;
    }
    vendor == NVIDIA_PCI_VENDOR || vendor == b"0x10de"
}

#[cfg(not(test))]
fn restart_with_safe_webkit_renderer() -> Result<(), String> {
    let wayland_session = env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
    let renderer_was_configured = env::var_os(WEBKIT_DMABUF_VARIABLE).is_some();
    if !should_restart_with_safe_webkit_renderer(
        wayland_session,
        nvidia_pci_device_present(Path::new(PCI_DEVICES_PATH)),
        renderer_was_configured,
    ) {
        return Ok(());
    }

    // WebKitGTK can violate explicit-sync protocol on NVIDIA/Wayland. Replacing
    // this process sets its documented renderer fallback before GTK starts,
    // without changing the user's session or spawning a second application.
    let executable = env::current_exe()
        .map_err(|error| format!("could not locate the Control Center executable: {error}"))?;
    let error = Command::new(executable)
        .args(env::args_os().skip(1))
        .env(WEBKIT_DMABUF_VARIABLE, "1")
        .exec();
    Err(format!(
        "could not restart with the NVIDIA/Wayland WebKit workaround: {error}"
    ))
}

#[cfg(not(test))]
fn main() {
    if let Some(status) = run_settings_diagnostic_request() {
        std::process::exit(status);
    }
    if let Err(error) = restart_with_safe_webkit_renderer() {
        eprintln!("OverCrow Control Center failed: {error}");
        std::process::exit(1);
    }

    let builder = tauri::Builder::default()
        .manage(CommandState::production())
        .manage(SupportReportState::default())
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
            commands::prepare_support_report,
            commands::submit_support_report,
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

#[cfg(test)]
mod startup_tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{nvidia_pci_device_present, should_restart_with_safe_webkit_renderer};

    #[test]
    fn restarts_only_for_unconfigured_nvidia_wayland_sessions() {
        assert!(should_restart_with_safe_webkit_renderer(true, true, false));
        assert!(!should_restart_with_safe_webkit_renderer(
            false, true, false
        ));
        assert!(!should_restart_with_safe_webkit_renderer(
            true, false, false
        ));
        assert!(!should_restart_with_safe_webkit_renderer(true, true, true));
    }

    #[test]
    fn detects_an_nvidia_pci_vendor_without_accepting_other_devices() {
        let root = tempdir().expect("temporary PCI root should be available");
        let amd = root.path().join("0000:01:00.0");
        let nvidia = root.path().join("0000:02:00.0");
        fs::create_dir(&amd).expect("AMD fixture directory should be created");
        fs::create_dir(&nvidia).expect("NVIDIA fixture directory should be created");
        fs::write(amd.join("vendor"), b"0x1002\n").expect("AMD fixture should be written");
        assert!(!nvidia_pci_device_present(root.path()));

        fs::write(nvidia.join("vendor"), b"0x10de\n").expect("NVIDIA fixture should be written");
        assert!(nvidia_pci_device_present(root.path()));
    }
}
