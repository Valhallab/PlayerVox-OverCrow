use std::time::Duration;

use tauri::{App, AppHandle, Manager};
use zbus::{blocking::Connection, interface};

use crate::tray;

const DBUS_NAME: &str = "com.playervox.OverCrow.SingleInstance";
const DBUS_PATH: &str = "/com/playervox/OverCrow/SingleInstance";
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(3);

struct PrimaryInstance {
    _connection: Connection,
}

struct SingleInstanceEndpoint {
    app: AppHandle,
}

#[interface(name = "com.playervox.OverCrow.SingleInstance")]
impl SingleInstanceEndpoint {
    fn show_control_center(&self) {
        tray::show_main_window(&self.app);
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Acquisition<T> {
    Primary(T),
    Secondary,
}

pub(crate) fn classify_acquisition<T>(result: zbus::Result<T>) -> Result<Acquisition<T>, String> {
    match result {
        Ok(connection) => Ok(Acquisition::Primary(connection)),
        Err(zbus::Error::NameTaken) => Ok(Acquisition::Secondary),
        Err(error) => Err(format!(
            "could not acquire the Control Center single-instance guard: {error}"
        )),
    }
}

pub(crate) fn install(app: &mut App) -> Result<(), String> {
    let endpoint = SingleInstanceEndpoint {
        app: app.handle().clone(),
    };
    let builder = zbus::blocking::connection::Builder::session()
        .map_err(|error| format!("could not connect the single-instance guard: {error}"))?
        .method_timeout(NOTIFY_TIMEOUT)
        .name(DBUS_NAME)
        .map_err(|error| format!("invalid single-instance bus name: {error}"))?
        .replace_existing_names(false)
        .allow_name_replacements(false)
        .serve_at(DBUS_PATH, endpoint)
        .map_err(|error| format!("invalid single-instance endpoint: {error}"))?;

    match classify_acquisition(builder.build())? {
        Acquisition::Primary(connection) => {
            app.manage(PrimaryInstance {
                _connection: connection,
            });
            Ok(())
        }
        Acquisition::Secondary => {
            notify_primary()?;
            app.cleanup_before_exit();
            std::process::exit(0);
        }
    }
}

fn notify_primary() -> Result<(), String> {
    let connection = zbus::blocking::connection::Builder::session()
        .map_err(|error| format!("could not connect to the existing Control Center: {error}"))?
        .method_timeout(NOTIFY_TIMEOUT)
        .build()
        .map_err(|error| format!("could not reach the existing Control Center: {error}"))?;
    connection
        .call_method(
            Some(DBUS_NAME),
            DBUS_PATH,
            Some(DBUS_NAME),
            "ShowControlCenter",
            &(),
        )
        .map(|_| ())
        .map_err(|error| format!("could not reopen the existing Control Center: {error}"))
}
