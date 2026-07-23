use std::ffi::OsString;

use overcrow_config::SettingsDiagnostic;

use crate::{
    SETTINGS_DIAGNOSTIC_ARG, encode_settings_diagnostic, settings_diagnostic_requested_from,
};

#[test]
fn settings_diagnostic_hidden_mode_requires_the_exact_argument() {
    assert!(settings_diagnostic_requested_from(["overcrow-control"]).is_ok_and(|value| !value));
    assert!(
        settings_diagnostic_requested_from(["overcrow-control", SETTINGS_DIAGNOSTIC_ARG])
            .is_ok_and(|value| value)
    );
    assert!(
        settings_diagnostic_requested_from(["overcrow-control", "--unknown"])
            .is_ok_and(|value| !value)
    );
    let extra = vec![
        OsString::from("overcrow-control"),
        OsString::from(SETTINGS_DIAGNOSTIC_ARG),
        OsString::from("extra"),
    ];
    assert!(settings_diagnostic_requested_from(extra).is_err());
}

#[test]
fn settings_diagnostic_protocol_is_small_exact_and_data_minimizing() {
    assert_eq!(
        encode_settings_diagnostic(SettingsDiagnostic::Missing),
        "missing|disabled|0\n"
    );
    assert_eq!(
        encode_settings_diagnostic(SettingsDiagnostic::Unavailable),
        "unavailable|unavailable|unavailable\n"
    );
    assert_eq!(
        encode_settings_diagnostic(SettingsDiagnostic::Invalid),
        "invalid|disabled|0\n"
    );
    assert_eq!(
        encode_settings_diagnostic(SettingsDiagnostic::Valid {
            enabled: true,
            selected_games: 3,
        }),
        "valid|enabled|3\n"
    );
}
