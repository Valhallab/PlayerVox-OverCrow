use overcrow_control::{
    CompatibilityReason, CompatibilityStatus, ControlCompatibility, ControlLifecycle,
    ControlNotice, ControlOperationState, ControlSnapshot, DesktopEnvironment, DisplaySession,
    NoticeLevelCode, NoticeOperationCode,
};

use crate::tray::{TrayPresentation, shutdown_succeeded};
use crate::{
    single_instance::{Acquisition, classify_acquisition},
    tray::LIFECYCLE_TIMEOUT,
};

fn snapshot(lifecycle: ControlLifecycle, action_enabled: bool) -> ControlSnapshot {
    ControlSnapshot {
        schema_version: overcrow_control::CONTROL_SNAPSHOT_SCHEMA_VERSION,
        compatibility: ControlCompatibility {
            operating_system: "Linux".to_owned(),
            session: DisplaySession::Wayland,
            desktop: DesktopEnvironment::Hyprland,
            status: CompatibilityStatus::Supported,
            reason: CompatibilityReason::HyprlandWayland,
            activation_allowed: true,
        },
        lifecycle,
        master_switch_enabled: action_enabled,
        master_switch_checked: matches!(
            lifecycle,
            ControlLifecycle::Enabled | ControlLifecycle::Enabling
        ),
        operations: ControlOperationState {
            lifecycle: matches!(
                lifecycle,
                ControlLifecycle::Enabling | ControlLifecycle::Disabling
            ),
            ..ControlOperationState::default()
        },
        selection_editing_enabled: true,
        shortcut: "Meta+Alt+O".to_owned(),
        games: Vec::new(),
        manual_games: Vec::new(),
        notices: Vec::new(),
        diagnostics: Vec::new(),
    }
}

#[test]
fn tray_presents_ready_state_as_a_start_action() {
    let presentation = TrayPresentation::from_snapshot(&snapshot(ControlLifecycle::Disabled, true));

    assert_eq!(presentation.status, "Status: Ready");
    assert_eq!(presentation.action, "Start OverCrow");
    assert!(presentation.action_enabled);
    assert_eq!(presentation.requested_state, Some(true));
}

#[test]
fn tray_presents_running_state_as_a_stop_action() {
    let presentation = TrayPresentation::from_snapshot(&snapshot(ControlLifecycle::Enabled, true));

    assert_eq!(presentation.status, "Status: Running");
    assert_eq!(presentation.action, "Stop OverCrow");
    assert!(presentation.action_enabled);
    assert_eq!(presentation.requested_state, Some(false));
}

#[test]
fn tray_disables_actions_during_transitions() {
    for (lifecycle, status, action) in [
        (
            ControlLifecycle::Enabling,
            "Status: Starting…",
            "Start OverCrow",
        ),
        (
            ControlLifecycle::Disabling,
            "Status: Stopping…",
            "Stop OverCrow",
        ),
    ] {
        let presentation = TrayPresentation::from_snapshot(&snapshot(lifecycle, true));

        assert_eq!(presentation.status, status);
        assert_eq!(presentation.action, action);
        assert!(!presentation.action_enabled);
        assert_eq!(presentation.requested_state, None);
    }
}

#[test]
fn tray_keeps_fail_closed_cleanup_available() {
    let warning = TrayPresentation::from_snapshot(&snapshot(ControlLifecycle::Warning, true));
    assert_eq!(warning.status, "Status: Attention required");
    assert_eq!(warning.action, "Stop OverCrow");
    assert!(warning.action_enabled);
    assert_eq!(warning.requested_state, Some(false));

    let mut failed_disable = snapshot(ControlLifecycle::Disabled, true);
    failed_disable.notices.push(ControlNotice {
        operation: NoticeOperationCode::Lifecycle,
        level: NoticeLevelCode::Error,
        message: "bounded fixture".to_owned(),
    });
    let failed = TrayPresentation::from_snapshot(&failed_disable);
    assert_eq!(failed.status, "Status: Attention required");
    assert_eq!(failed.action, "Stop OverCrow");
    assert!(failed.action_enabled);
    assert_eq!(failed.requested_state, Some(false));
}

#[test]
fn tray_keeps_start_disabled_until_activation_is_allowed() {
    let presentation =
        TrayPresentation::from_snapshot(&snapshot(ControlLifecycle::Disabled, false));

    assert_eq!(presentation.status, "Status: Ready");
    assert_eq!(presentation.action, "Start OverCrow");
    assert!(!presentation.action_enabled);
    assert_eq!(presentation.requested_state, None);
}

#[test]
fn quit_requires_a_clean_completed_disable_transaction() {
    let disabled = snapshot(ControlLifecycle::Disabled, true);
    assert!(shutdown_succeeded(&disabled));
    assert!(!shutdown_succeeded(&snapshot(
        ControlLifecycle::Disabling,
        false,
    )));

    let mut failed = disabled;
    failed.notices.push(ControlNotice {
        operation: NoticeOperationCode::Lifecycle,
        level: NoticeLevelCode::Error,
        message: "bounded fixture".to_owned(),
    });
    assert!(!shutdown_succeeded(&failed));
}

#[test]
fn single_instance_acquisition_fails_closed_except_for_an_owned_name() {
    assert_eq!(
        classify_acquisition::<()>(Err(zbus::Error::NameTaken)),
        Ok(Acquisition::Secondary)
    );

    let unavailable =
        classify_acquisition::<()>(Err(zbus::Error::Address("unavailable test bus".to_owned())));
    assert!(unavailable.is_err());
}

#[test]
fn lifecycle_monitor_covers_the_bounded_enable_and_rollback_budget() {
    assert!(LIFECYCLE_TIMEOUT >= std::time::Duration::from_secs(120));
}
