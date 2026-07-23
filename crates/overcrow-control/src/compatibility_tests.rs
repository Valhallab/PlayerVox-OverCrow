use crate::compatibility::environment_identity_from_sources;
use crate::{
    CompatibilityReason, CompatibilityReport, CompatibilityStatus, DesktopEnvironment,
    DisplaySession, EnvironmentIdentity, MAX_ENVIRONMENT_LABEL_BYTES,
};

fn identity(session: &str, desktop: &str, desktop_session: &str) -> EnvironmentIdentity {
    EnvironmentIdentity {
        session_type: Some(session.to_owned()),
        current_desktop: Some(desktop.to_owned()),
        desktop_session: Some(desktop_session.to_owned()),
        os_name: Some("Arch Linux".to_owned()),
    }
}

#[test]
fn hyprland_wayland_is_the_supported_primary_target() {
    let report = CompatibilityReport::from_environment(identity("wayland", "Hyprland", "omarchy"));

    assert_eq!(report.session, DisplaySession::Wayland);
    assert_eq!(report.desktop, DesktopEnvironment::Hyprland);
    assert_eq!(report.status, CompatibilityStatus::Supported);
    assert_eq!(report.reason, CompatibilityReason::HyprlandWayland);
    assert!(report.activation_allowed);
    assert_eq!(report.operating_system, "Arch Linux");
}

#[test]
fn plasma_wayland_is_available_while_validation_continues() {
    for desktop in ["KDE", "plasma", "KDE:Plasma"] {
        let report =
            CompatibilityReport::from_environment(identity("WAYLAND", desktop, "plasmawayland"));

        assert_eq!(report.desktop, DesktopEnvironment::Plasma);
        assert_eq!(report.status, CompatibilityStatus::ValidationInProgress);
        assert_eq!(report.reason, CompatibilityReason::PlasmaWayland);
        assert!(report.activation_allowed);
    }
}

#[test]
fn generic_x11_is_experimental_for_now_but_eligible() {
    let report = CompatibilityReport::from_environment(identity("x11", "i3", "i3"));

    assert_eq!(report.session, DisplaySession::X11);
    assert_eq!(report.status, CompatibilityStatus::ExperimentalForNow);
    assert_eq!(report.reason, CompatibilityReason::GenericX11);
    assert!(report.activation_allowed);
}

#[test]
fn known_unsupported_desktops_fail_closed_for_now() {
    let cases = [
        (
            "wayland",
            "GNOME",
            "gnome",
            DesktopEnvironment::Gnome,
            CompatibilityReason::GnomeWayland,
        ),
        (
            "wayland",
            "sway",
            "sway",
            DesktopEnvironment::Sway,
            CompatibilityReason::SwayWayland,
        ),
        (
            "wayland",
            "gamescope",
            "gamescope",
            DesktopEnvironment::Gamescope,
            CompatibilityReason::GamescopeSession,
        ),
        (
            "x11",
            "XFCE",
            "xfce",
            DesktopEnvironment::Xfce,
            CompatibilityReason::XfceX11,
        ),
    ];

    for (session, current, desktop_session, expected_desktop, expected_reason) in cases {
        let report =
            CompatibilityReport::from_environment(identity(session, current, desktop_session));
        assert_eq!(report.desktop, expected_desktop);
        assert_eq!(report.status, CompatibilityStatus::NotCompatibleForNow);
        assert_eq!(report.reason, expected_reason);
        assert!(!report.activation_allowed);
    }
}

#[test]
fn unknown_wayland_and_unknown_sessions_fail_closed() {
    let wayland =
        CompatibilityReport::from_environment(identity("wayland", "MangoCompositor", "mango"));
    assert_eq!(wayland.desktop, DesktopEnvironment::Other);
    assert_eq!(wayland.status, CompatibilityStatus::NotCompatibleForNow);
    assert_eq!(wayland.reason, CompatibilityReason::OtherWayland);
    assert!(!wayland.activation_allowed);

    let unknown = CompatibilityReport::from_environment(identity("tty", "", ""));
    assert_eq!(unknown.session, DisplaySession::Unknown);
    assert_eq!(unknown.desktop, DesktopEnvironment::Unknown);
    assert_eq!(unknown.status, CompatibilityStatus::Unknown);
    assert_eq!(unknown.reason, CompatibilityReason::UnknownSession);
    assert!(!unknown.activation_allowed);
}

#[test]
fn conflicting_desktop_metadata_is_ambiguous_and_fails_closed() {
    let report = CompatibilityReport::from_environment(identity("wayland", "GNOME:KDE", "plasma"));

    assert_eq!(report.desktop, DesktopEnvironment::Ambiguous);
    assert_eq!(report.status, CompatibilityStatus::Unknown);
    assert_eq!(report.reason, CompatibilityReason::AmbiguousDesktop);
    assert!(!report.activation_allowed);
}

#[test]
fn environment_labels_are_trimmed_and_utf8_bounded() {
    let report = CompatibilityReport::from_environment(EnvironmentIdentity {
        session_type: Some(" wayland ".to_owned()),
        current_desktop: Some(" Hyprland ".to_owned()),
        desktop_session: None,
        os_name: Some(format!("{}é", "A".repeat(MAX_ENVIRONMENT_LABEL_BYTES * 2))),
    });

    assert!(report.operating_system.len() <= MAX_ENVIRONMENT_LABEL_BYTES);
    assert!(
        report
            .operating_system
            .is_char_boundary(report.operating_system.len())
    );
    assert!(
        !report
            .operating_system
            .ends_with(char::REPLACEMENT_CHARACTER)
    );
}

#[test]
fn desktop_substrings_cannot_impersonate_supported_targets() {
    for desktop in ["not-hyprland", "kde-backup", "mygnome", "swayfx-extra"] {
        let report = CompatibilityReport::from_environment(identity("wayland", desktop, desktop));
        assert_eq!(report.desktop, DesktopEnvironment::Other);
        assert_eq!(report.status, CompatibilityStatus::NotCompatibleForNow);
    }
}

#[test]
fn environment_identity_reads_bounded_os_release_without_shell_evaluation() {
    let identity = environment_identity_from_sources(
        |name| match name {
            "XDG_SESSION_TYPE" => Some("wayland".into()),
            "XDG_CURRENT_DESKTOP" => Some("Hyprland".into()),
            "DESKTOP_SESSION" => Some("omarchy".into()),
            _ => None,
        },
        Some(b"NAME=Arch\nPRETTY_NAME=\"Arch Linux \\\"Rolling\\\"\"\n"),
    );

    assert_eq!(identity.session_type.as_deref(), Some("wayland"));
    assert_eq!(identity.current_desktop.as_deref(), Some("Hyprland"));
    assert_eq!(identity.desktop_session.as_deref(), Some("omarchy"));
    assert_eq!(identity.os_name.as_deref(), Some("Arch Linux \"Rolling\""));
}

#[test]
fn malformed_or_oversized_os_release_is_not_presented() {
    for source in [
        b"PRETTY_NAME=\"unterminated\n".as_slice(),
        vec![b'A'; 32 * 1024].leak(),
    ] {
        let identity = environment_identity_from_sources(|_| None, Some(source));
        assert_eq!(identity.os_name, None);
    }
}
