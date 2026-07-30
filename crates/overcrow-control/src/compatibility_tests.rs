use std::{fs, os::unix::fs::symlink};

use tempfile::tempdir;

use crate::compatibility::{
    MAX_PCI_DEVICES, detect_graphics_vendors, detect_graphics_vendors_from_paths,
    environment_identity_from_sources,
};
use crate::{
    CompatibilityReason, CompatibilityReport, CompatibilityStatus, DesktopEnvironment,
    DisplaySession, EnvironmentIdentity, GraphicsVendor, MAX_ENVIRONMENT_LABEL_BYTES,
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
    for (desktop, expected_desktop) in [
        ("i3", DesktopEnvironment::Other),
        ("XFCE", DesktopEnvironment::Xfce),
    ] {
        let report = CompatibilityReport::from_environment(identity("x11", desktop, desktop));

        assert_eq!(report.session, DisplaySession::X11);
        assert_eq!(report.desktop, expected_desktop);
        assert_eq!(report.status, CompatibilityStatus::ExperimentalForNow);
        assert_eq!(report.reason, CompatibilityReason::GenericX11);
        assert!(report.activation_allowed);
    }
}

#[test]
fn gnome_wayland_is_experimental_for_now_but_eligible() {
    for (current_desktop, desktop_session) in [
        ("GNOME", "gnome"),
        ("gnome", "ubuntu"),
        ("ubuntu:GNOME", "ubuntu-wayland"),
        ("", "ubuntu-wayland"),
    ] {
        let report = CompatibilityReport::from_environment(identity(
            "wayland",
            current_desktop,
            desktop_session,
        ));

        assert_eq!(report.desktop, DesktopEnvironment::Gnome);
        assert_eq!(report.status, CompatibilityStatus::ExperimentalForNow);
        assert_eq!(report.reason, CompatibilityReason::GnomeWayland);
        assert!(report.activation_allowed);
    }
}

#[test]
fn known_unsupported_desktops_fail_closed_for_now() {
    let cases = [
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

#[test]
fn graphics_detection_keeps_only_bounded_display_controller_vendors() {
    let root = tempdir().expect("temporary PCI root");
    for (name, class, vendor) in [
        ("0000:00:02.0", "0x030000\n", "0x8086\n"),
        ("0000:01:00.0", "0x030200\n", "0x10de\n"),
        ("0000:02:00.0", "0x030000\n", "0x1002\n"),
        ("0000:03:00.0", "0x020000\n", "0x10de\n"),
        ("0000:04:00.0", "not-a-class\n", "0x10de\n"),
        ("0000:05:00.0", "0x038000\n", "0x1234\n"),
        ("0000:06:00.0", "0x030000\n", "0x10de\n"),
    ] {
        let device = root.path().join(name);
        fs::create_dir(&device).expect("PCI fixture directory");
        fs::write(device.join("class"), class).expect("PCI class fixture");
        fs::write(device.join("vendor"), vendor).expect("PCI vendor fixture");
    }

    assert_eq!(
        detect_graphics_vendors(root.path()),
        vec![
            GraphicsVendor::Amd,
            GraphicsVendor::Intel,
            GraphicsVendor::Nvidia,
            GraphicsVendor::Other,
        ]
    );
}

#[test]
fn graphics_detection_rejects_symlinked_and_oversized_metadata() {
    let root = tempdir().expect("temporary PCI root");
    let outside = root.path().join("outside");
    fs::write(&outside, "0x030000\n").expect("outside fixture");

    let symlinked = root.path().join("0000:01:00.0");
    fs::create_dir(&symlinked).expect("symlinked fixture directory");
    symlink(&outside, symlinked.join("class")).expect("class symlink");
    fs::write(symlinked.join("vendor"), "0x10de\n").expect("vendor fixture");

    let oversized = root.path().join("0000:02:00.0");
    fs::create_dir(&oversized).expect("oversized fixture directory");
    fs::write(oversized.join("class"), "0x030000\n").expect("class fixture");
    fs::write(oversized.join("vendor"), "0x10de".repeat(32)).expect("oversized vendor fixture");

    assert!(detect_graphics_vendors(root.path()).is_empty());
}

#[test]
fn graphics_detection_stops_at_the_device_scan_limit() {
    let root = tempdir().expect("temporary PCI root");
    let mut paths = Vec::with_capacity(MAX_PCI_DEVICES + 1);
    for index in 0..MAX_PCI_DEVICES {
        let device = root.path().join(format!("{index:04x}:00:00.0"));
        fs::create_dir(&device).expect("non-display fixture directory");
        paths.push(device);
    }
    let late = root.path().join("ffff:00:00.0");
    fs::create_dir(&late).expect("late fixture directory");
    fs::write(late.join("class"), "0x030000\n").expect("late class fixture");
    fs::write(late.join("vendor"), "0x10de\n").expect("late vendor fixture");
    paths.push(late);

    assert!(detect_graphics_vendors_from_paths(paths).is_empty());
}

#[test]
fn compatibility_report_normalizes_graphics_vendors_without_changing_activation() {
    let report = CompatibilityReport::from_environment(identity("wayland", "Hyprland", "omarchy"))
        .with_graphics(vec![
            GraphicsVendor::Nvidia,
            GraphicsVendor::Intel,
            GraphicsVendor::Nvidia,
        ]);

    assert_eq!(
        report.graphics,
        vec![GraphicsVendor::Intel, GraphicsVendor::Nvidia]
    );
    assert!(report.activation_allowed);
}
