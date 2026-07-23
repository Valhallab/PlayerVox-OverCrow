mod app;
mod branding;
mod media;
mod notes;
mod placement;
mod preferences;
mod runtime;
mod session_clock;
mod warframe;
pub mod widgets;

use app::{APP_ID, OverlayApp, is_x11_session, viewport_builder};
use overcrow_config::{SettingsLoad, SettingsStore};
use overcrow_logging::{Component, EventLogger, LoggerRuntime};

fn main() -> eframe::Result {
    let settings_load = SettingsStore::from_environment().load();
    if let Some(warning) = &settings_load.warning {
        eprintln!("OverCrow lifecycle settings rejected; remaining inert: {warning}");
    }

    run_if_lifecycle_allows(&settings_load, run_overlay)
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
    logger.info(
        "process_started",
        format_args!("x11_session={}", is_x11_session()),
    );
    let options = eframe::NativeOptions {
        viewport: viewport_builder(is_x11_session()),
        persist_window: false,
        ..Default::default()
    };
    let app_logger = logger.clone();
    let result = eframe::run_native(
        APP_ID,
        options,
        Box::new(move |creation_context| {
            Ok(Box::new(OverlayApp::new(creation_context, app_logger)))
        }),
    );
    logger.info("process_stopping", format_args!("reason=event_loop_ended"));
    result
}

fn lifecycle_allows_start(load: &SettingsLoad) -> bool {
    load.warning.is_none() && load.settings.enabled && load.settings.clone().validate().is_ok()
}

fn run_if_lifecycle_allows<E>(
    load: &SettingsLoad,
    start: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    if !lifecycle_allows_start(load) {
        return Ok(());
    }

    start()
}

#[cfg(test)]
mod tests {
    use overcrow_config::{LifecycleSettings, SettingsLoad};

    use super::{lifecycle_allows_start, run_if_lifecycle_allows};

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

        run_if_lifecycle_allows(&settings_load(false), || {
            entered = true;
            Ok::<_, ()>(())
        })
        .unwrap();

        assert!(!entered);
    }
}
