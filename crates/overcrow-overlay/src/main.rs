mod app;
mod branding;
mod discord;
mod icons;
mod media;
mod notes;
mod placement;
mod preferences;
mod runtime;
mod session_clock;
pub mod twitch;
mod warframe;
pub mod widgets;

use std::process::ExitCode;

use app::{APP_ID, OverlayApp, is_x11_session, viewport_builder};
use overcrow_config::{SettingsLoad, SettingsStore};
use overcrow_logging::{Component, EventLogger, LoggerRuntime};

fn main() -> ExitCode {
    let settings_load = SettingsStore::from_environment().load();
    if let Some(warning) = &settings_load.warning {
        eprintln!("OverCrow lifecycle settings rejected; remaining inert: {warning}");
    }

    match run_if_lifecycle_allows(&settings_load, run_overlay) {
        Ok(false) => ExitCode::SUCCESS,
        Ok(true) => {
            eprintln!("OverCrow overlay event loop ended unexpectedly");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("OverCrow overlay failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_overlay() -> eframe::Result {
    let log_runtime = match LoggerRuntime::start(Component::Overlay) {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            eprintln!("OverCrow diagnostic logger failed to start: {error}");
            None
        }
    };
    let logger = log_runtime
        .as_ref()
        .map(LoggerRuntime::logger)
        .unwrap_or_else(EventLogger::disabled);
    let x11_session = is_x11_session();
    logger.info("process_started", format_args!("x11_session={x11_session}"));
    let options = native_options(x11_session);
    let app_logger = logger.clone();
    let result = eframe::run_native(
        APP_ID,
        options,
        Box::new(move |creation_context| {
            Ok(Box::new(OverlayApp::new(
                creation_context,
                app_logger,
                x11_session,
            )))
        }),
    );
    logger.info("process_stopping", format_args!("reason=event_loop_ended"));
    result
}

fn native_options(x11_session: bool) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: viewport_builder(x11_session),
        persist_window: false,
        glow_options: eframe::egui_glow::GlowConfiguration {
            // Wayland compositors may stop scheduling hidden surfaces. Waiting
            // for VSync there can block Wayland ping handling and trigger ANR.
            vsync: x11_session,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn lifecycle_allows_start(load: &SettingsLoad) -> bool {
    load.warning.is_none() && load.settings.enabled && load.settings.clone().validate().is_ok()
}

fn run_if_lifecycle_allows<E>(
    load: &SettingsLoad,
    start: impl FnOnce() -> Result<(), E>,
) -> Result<bool, E> {
    if !lifecycle_allows_start(load) {
        return Ok(false);
    }

    start().map(|()| true)
}

#[cfg(test)]
mod tests {
    use overcrow_config::{LifecycleSettings, SettingsLoad};

    use super::{lifecycle_allows_start, native_options, run_if_lifecycle_allows};

    fn settings_load(enabled: bool) -> SettingsLoad {
        SettingsLoad {
            settings: LifecycleSettings {
                enabled,
                ..LifecycleSettings::default()
            },
            warning: None,
        }
    }

    #[test]
    fn lifecycle_guard_accepts_only_valid_enabled_settings() {
        assert!(lifecycle_allows_start(&settings_load(true)));
        assert!(!lifecycle_allows_start(&settings_load(false)));

        let mut warned = settings_load(true);
        warned.warning = Some("unsafe settings".to_owned());
        assert!(!lifecycle_allows_start(&warned));

        let mut invalid = settings_load(true);
        invalid.settings.schema_version += 1;
        assert!(!lifecycle_allows_start(&invalid));
    }

    #[test]
    fn rejected_settings_do_not_create_the_overlay() {
        let mut entered = false;

        let started = run_if_lifecycle_allows(&settings_load(false), || {
            entered = true;
            Ok::<_, ()>(())
        })
        .unwrap();

        assert!(!started);
        assert!(!entered);
    }

    #[test]
    fn an_unexpected_successful_event_loop_return_is_reported_as_started() {
        assert!(
            run_if_lifecycle_allows(&settings_load(true), || Ok::<_, ()>(()))
                .expect("event loop result")
        );
    }

    #[test]
    fn wayland_disables_vsync_while_x11_keeps_it() {
        assert!(!native_options(false).glow_options.vsync);
        assert!(native_options(true).glow_options.vsync);
    }
}
