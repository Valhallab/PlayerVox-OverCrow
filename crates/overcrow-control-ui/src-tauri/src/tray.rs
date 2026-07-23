use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use overcrow_control::{ControlLifecycle, ControlSnapshot, NoticeLevelCode, NoticeOperationCode};
use tauri::{
    App, AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

use crate::commands::CommandState;

const CONTROL_STATE_EVENT: &str = "overcrow-control-state";
const STATUS_ID: &str = "overcrow-status";
const ACTION_ID: &str = "overcrow-action";
const OPEN_ID: &str = "overcrow-open";
const QUIT_ID: &str = "overcrow-quit";
const MAIN_WINDOW_LABEL: &str = "main";
const MONITOR_INTERVAL: Duration = Duration::from_millis(100);
// Covers activation preflight, integration, Core startup, and bounded rollback.
pub(crate) const LIFECYCLE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TrayPresentation {
    pub(crate) status: &'static str,
    pub(crate) action: &'static str,
    pub(crate) action_enabled: bool,
    pub(crate) requested_state: Option<bool>,
}

impl TrayPresentation {
    pub(crate) fn from_snapshot(snapshot: &ControlSnapshot) -> Self {
        if has_lifecycle_error(snapshot) {
            return Self {
                status: "Status: Attention required",
                action: "Stop OverCrow",
                action_enabled: !snapshot.operations.lifecycle,
                requested_state: (!snapshot.operations.lifecycle).then_some(false),
            };
        }
        match snapshot.lifecycle {
            ControlLifecycle::Disabled => Self {
                status: "Status: Ready",
                action: "Start OverCrow",
                action_enabled: snapshot.master_switch_enabled,
                requested_state: snapshot.master_switch_enabled.then_some(true),
            },
            ControlLifecycle::Enabled => Self {
                status: "Status: Running",
                action: "Stop OverCrow",
                action_enabled: true,
                requested_state: Some(false),
            },
            ControlLifecycle::Enabling => Self {
                status: "Status: Starting…",
                action: "Start OverCrow",
                action_enabled: false,
                requested_state: None,
            },
            ControlLifecycle::Disabling => Self {
                status: "Status: Stopping…",
                action: "Stop OverCrow",
                action_enabled: false,
                requested_state: None,
            },
            ControlLifecycle::Warning => Self {
                status: "Status: Attention required",
                action: "Stop OverCrow",
                action_enabled: !snapshot.operations.lifecycle,
                requested_state: (!snapshot.operations.lifecycle).then_some(false),
            },
        }
    }
}

pub(crate) fn shutdown_succeeded(snapshot: &ControlSnapshot) -> bool {
    snapshot.lifecycle == ControlLifecycle::Disabled && !has_lifecycle_error(snapshot)
}

fn has_lifecycle_error(snapshot: &ControlSnapshot) -> bool {
    snapshot.notices.iter().any(|notice| {
        notice.operation == NoticeOperationCode::Lifecycle && notice.level == NoticeLevelCode::Error
    })
}

struct TrayState {
    status: MenuItem<tauri::Wry>,
    action: MenuItem<tauri::Wry>,
    lifecycle_monitor_running: AtomicBool,
    quitting: AtomicBool,
}

pub(crate) fn install(app: &mut App) -> tauri::Result<()> {
    let snapshot = app
        .state::<CommandState>()
        .get_control_state()
        .map_err(|error| tauri::Error::Io(std::io::Error::other(error)))?;
    let presentation = TrayPresentation::from_snapshot(&snapshot);
    let status = MenuItem::with_id(app, STATUS_ID, presentation.status, false, None::<&str>)?;
    let action = MenuItem::with_id(
        app,
        ACTION_ID,
        presentation.action,
        presentation.action_enabled,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let open = MenuItem::with_id(app, OPEN_ID, "Open Control Center", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &action, &separator, &open, &quit])?;

    app.manage(TrayState {
        status,
        action,
        lifecycle_monitor_running: AtomicBool::new(false),
        quitting: AtomicBool::new(false),
    });

    let mut tray = TrayIconBuilder::with_id("overcrow")
        .menu(&menu)
        .tooltip("PlayerVox OverCrow")
        .on_menu_event(handle_menu_event);
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

pub(crate) fn publish_snapshot(app: &AppHandle, snapshot: &ControlSnapshot) {
    sync_snapshot(app, snapshot);
    if let Err(error) = app.emit(CONTROL_STATE_EVENT, snapshot) {
        eprintln!("OverCrow could not publish the Control Center state: {error}");
    }
}

pub(crate) fn sync_snapshot(app: &AppHandle, snapshot: &ControlSnapshot) {
    // A second launch can reach the single-instance callback while the first
    // process is still completing setup.
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    if state.quitting.load(Ordering::Acquire) {
        return;
    }
    let presentation = TrayPresentation::from_snapshot(snapshot);
    if let Err(error) = state.status.set_text(presentation.status) {
        eprintln!("OverCrow could not update the tray status: {error}");
    }
    if let Err(error) = state.action.set_text(presentation.action) {
        eprintln!("OverCrow could not update the tray action: {error}");
    }
    if let Err(error) = state.action.set_enabled(presentation.action_enabled) {
        eprintln!("OverCrow could not update the tray availability: {error}");
    }
}

pub(crate) fn ensure_lifecycle_monitor(app: &AppHandle) {
    let state = app.state::<TrayState>();
    if state.lifecycle_monitor_running.swap(true, Ordering::AcqRel) {
        return;
    }

    // Lifecycle work already runs on the Control Center worker. This bounded
    // monitor only keeps the tray current while the webview is hidden.
    let app = app.clone();
    if let Err(error) = thread::Builder::new()
        .name("overcrow-tray-lifecycle".to_owned())
        .spawn(move || monitor_lifecycle(app))
    {
        state
            .lifecycle_monitor_running
            .store(false, Ordering::Release);
        eprintln!("OverCrow could not monitor the lifecycle action: {error}");
    }
}

pub(crate) fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    if let Err(error) = window.unminimize() {
        eprintln!("OverCrow could not restore the Control Center: {error}");
    }
    if let Err(error) = window.show() {
        eprintln!("OverCrow could not show the Control Center: {error}");
        return;
    }
    if let Err(error) = window.set_focus() {
        eprintln!("OverCrow could not focus the Control Center: {error}");
    }
    if let Ok(snapshot) = app.state::<CommandState>().get_control_state() {
        publish_snapshot(app, &snapshot);
    }
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        ACTION_ID => request_toggle(app),
        OPEN_ID => show_main_window(app),
        QUIT_ID => request_quit(app),
        _ => {}
    }
}

fn request_toggle(app: &AppHandle) {
    let commands = app.state::<CommandState>();
    let Ok(snapshot) = commands.get_control_state() else {
        show_main_window(app);
        return;
    };
    let Some(requested) = TrayPresentation::from_snapshot(&snapshot).requested_state else {
        return;
    };
    match commands.set_overcrow_enabled(requested) {
        Ok(snapshot) => {
            publish_snapshot(app, &snapshot);
            ensure_lifecycle_monitor(app);
        }
        Err(_) => show_main_window(app),
    }
}

fn request_quit(app: &AppHandle) {
    let state = app.state::<TrayState>();
    if state.quitting.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = state.status.set_text("Status: Stopping…");
    let _ = state.action.set_enabled(false);

    let worker_app = app.clone();
    if let Err(error) = thread::Builder::new()
        .name("overcrow-tray-quit".to_owned())
        .spawn(move || stop_and_quit(worker_app))
    {
        state.quitting.store(false, Ordering::Release);
        eprintln!("OverCrow could not start the shutdown action: {error}");
        if let Ok(snapshot) = app.state::<CommandState>().get_control_state() {
            publish_snapshot(app, &snapshot);
        }
    }
}

fn monitor_lifecycle(app: AppHandle) {
    let result = wait_for_lifecycle(&app);
    app.state::<TrayState>()
        .lifecycle_monitor_running
        .store(false, Ordering::Release);
    match result {
        Ok(snapshot) => publish_snapshot(&app, &snapshot),
        Err(error) => {
            eprintln!("OverCrow lifecycle monitoring failed: {error}");
            show_main_window(&app);
        }
    }
}

fn stop_and_quit(app: AppHandle) {
    // Finish any active transition before requesting the same fail-closed
    // disable transaction used by the Control Center.
    if stop_runtime_for_quit(&app).is_ok() {
        app.exit(0);
        return;
    }

    app.state::<TrayState>()
        .quitting
        .store(false, Ordering::Release);
    if let Ok(snapshot) = app.state::<CommandState>().get_control_state() {
        publish_snapshot(&app, &snapshot);
    }
    show_main_window(&app);
}

fn stop_runtime_for_quit(app: &AppHandle) -> Result<(), &'static str> {
    wait_for_lifecycle(app)?;
    app.state::<CommandState>()
        .set_overcrow_enabled(false)
        .map_err(|_| "disable request rejected")?;
    let snapshot = wait_for_lifecycle(app)?;
    shutdown_succeeded(&snapshot)
        .then_some(())
        .ok_or("disable transaction failed")
}

fn wait_for_lifecycle(app: &AppHandle) -> Result<ControlSnapshot, &'static str> {
    let deadline = Instant::now() + LIFECYCLE_TIMEOUT;
    let mut last_published = None;
    loop {
        let snapshot = app
            .state::<CommandState>()
            .get_control_state()
            .map_err(|_| "state unavailable")?;
        if last_published != Some(snapshot.lifecycle) {
            publish_snapshot(app, &snapshot);
            last_published = Some(snapshot.lifecycle);
        }
        if !snapshot.operations.lifecycle {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err("action timed out");
        }
        thread::sleep(MONITOR_INTERVAL);
    }
}
