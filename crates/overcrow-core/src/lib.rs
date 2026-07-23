pub mod classifier;
mod event_log;
pub mod hwmon;
pub mod manual_stopwatch;
pub mod procfs_source;
pub mod runtime;
pub mod service;
pub mod session;
#[cfg(test)]
mod session_tests;
pub mod shortcut;
#[cfg(test)]
mod shortcut_tests;
pub mod snapshot_signal;
pub mod telemetry;
#[cfg(test)]
mod telemetry_tests;
pub mod window;
pub mod x11;

pub use classifier::{
    ProcessClassification, ProcessIdentity, ProcessInfo, ProcessResources, ProcessTiming,
    classify_active_pid, classify_process_identity,
};
pub use event_log::run_core_event_logging;
pub use hwmon::{TemperatureSnapshot, scan_temperatures};
pub use manual_stopwatch::ManualStopwatch;
pub use procfs_source::scan_processes;
pub use runtime::{
    BRIDGE_LEASE_TIMEOUT, BRIDGE_WATCHDOG_INTERVAL, CoreRuntime, OVERLAY_APP_ID,
    PROCESS_REFRESH_INTERVAL, RuntimeSettingsError, run_bridge_watchdog, run_process_refresh,
};
pub use service::{
    CoreService, WIDGET_SETTINGS_REFRESH_INTERVAL, WINDOW_POLL_INTERVAL, apply_window_observation,
    poll_window_once, run_widget_settings_refresh, run_window_polling, should_use_x11_source,
};
pub use session::{
    AppliedState, CommandFuture, CommandRunner, DesktopSession, SESSION_SHUTDOWN_TIMEOUT,
    SessionCommand, SessionCoordinator, SystemctlRunner, run_session_coordinator,
    shutdown_session_coordinator, transition_commands,
};
pub use shortcut::{
    PortalShortcutBroker, SHORTCUT_SHUTDOWN_TIMEOUT, ShortcutAvailability, ShortcutError,
    ShortcutEvent, ShortcutFuture, ShortcutPolicy, ShortcutPortal, ShortcutSession, XdgPortal,
    portal_trigger,
};
pub use snapshot_signal::{
    DbusSnapshotSignalSink, SignalFuture, SnapshotSignalSink, run_snapshot_signal_publisher,
};
pub use telemetry::TelemetrySampler;
pub(crate) use telemetry::collect_process_sample;
pub use window::{NoopWindowSource, WindowObservation, WindowSource};
pub use x11::X11WindowSource;
