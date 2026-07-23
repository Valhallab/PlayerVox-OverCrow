use std::{env, ffi::OsString, io};

use overcrow_config::{SettingsDiagnostic, SettingsStore};

pub const SETTINGS_DIAGNOSTIC_ARG: &str = "--overcrow-diagnose-settings-v1";

pub fn settings_diagnostic_requested_from<I, S>(arguments: I) -> io::Result<bool>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut arguments = arguments.into_iter().map(Into::into);
    let _program = arguments.next();
    let Some(argument) = arguments.next() else {
        return Ok(false);
    };
    if argument != SETTINGS_DIAGNOSTIC_ARG {
        return Ok(false);
    }
    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "settings diagnostic mode accepts no additional arguments",
        ));
    }
    Ok(true)
}

pub fn encode_settings_diagnostic(diagnostic: SettingsDiagnostic) -> String {
    match diagnostic {
        SettingsDiagnostic::Unavailable => "unavailable|unavailable|unavailable\n".to_owned(),
        SettingsDiagnostic::Missing => "missing|disabled|0\n".to_owned(),
        SettingsDiagnostic::Invalid => "invalid|disabled|0\n".to_owned(),
        SettingsDiagnostic::Valid {
            enabled,
            selected_games,
        } => format!(
            "valid|{}|{selected_games}\n",
            if enabled { "enabled" } else { "disabled" }
        ),
    }
}

pub fn run_settings_diagnostic_request() -> Option<i32> {
    match settings_diagnostic_requested_from(env::args_os()) {
        Ok(false) => None,
        Err(error) => {
            eprintln!("invalid settings diagnostic invocation: {error}");
            Some(2)
        }
        Ok(true) => {
            print!(
                "{}",
                encode_settings_diagnostic(SettingsStore::from_environment().diagnose())
            );
            Some(0)
        }
    }
}
