#[zbus::proxy(
    interface = "io.github.overcrow.Core1",
    default_service = "io.github.overcrow.Core1",
    default_path = "/io/github/overcrow/Core1"
)]
pub trait Core1 {
    #[zbus(name = "Snapshot")]
    fn snapshot(&self) -> zbus::Result<String>;

    #[zbus(name = "SnapshotVersioned")]
    fn snapshot_versioned(&self) -> zbus::Result<String>;

    #[zbus(signal, name = "SnapshotChanged")]
    fn snapshot_changed(&self, snapshot_json: &str) -> zbus::Result<()>;

    #[zbus(name = "ToggleOverlay")]
    fn toggle_overlay(&self) -> zbus::Result<String>;

    #[zbus(name = "SetOverlayInteractive")]
    fn set_overlay_interactive(&self, interactive: bool) -> zbus::Result<String>;

    #[zbus(name = "ReportWindow")]
    #[allow(clippy::too_many_arguments)]
    fn report_window(
        &self,
        pid: i32,
        title: &str,
        app_id: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        scale: &str,
    ) -> zbus::Result<String>;

    #[zbus(name = "ClearWindow")]
    fn clear_window(&self) -> zbus::Result<String>;

    #[zbus(name = "ReloadSettings")]
    fn reload_settings(&self) -> zbus::Result<String>;

    #[zbus(name = "ReloadWidgetSettings")]
    fn reload_widget_settings(&self) -> zbus::Result<String>;

    #[zbus(name = "ToggleManualStopwatch")]
    fn toggle_manual_stopwatch(&self) -> zbus::Result<String>;

    #[zbus(name = "ResetManualStopwatch")]
    fn reset_manual_stopwatch(&self) -> zbus::Result<String>;

    #[zbus(name = "ShortcutAvailability")]
    fn shortcut_availability(&self) -> zbus::Result<String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::proxy::Defaults;

    #[test]
    fn core_proxy_uses_the_fixed_bus_identity() {
        assert_eq!(
            Core1Proxy::DESTINATION
                .as_ref()
                .expect("proxy has a destination")
                .as_str(),
            "io.github.overcrow.Core1"
        );
        assert_eq!(
            Core1Proxy::PATH
                .as_ref()
                .expect("proxy has a path")
                .as_str(),
            "/io/github/overcrow/Core1"
        );
        assert_eq!(
            Core1Proxy::INTERFACE
                .as_ref()
                .expect("proxy has an interface")
                .as_str(),
            "io.github.overcrow.Core1"
        );
    }

    #[test]
    fn core_proxy_exposes_the_core1_methods() {
        fn assert_methods(proxy: &Core1Proxy<'_>) {
            std::mem::drop(proxy.snapshot());
            std::mem::drop(proxy.snapshot_versioned());
            std::mem::drop(proxy.receive_snapshot_changed());
            std::mem::drop(proxy.toggle_overlay());
            std::mem::drop(proxy.set_overlay_interactive(true));
            std::mem::drop(proxy.report_window(42, "Portal 2", "portal2", 0, 0, 1920, 1080, "1"));
            std::mem::drop(proxy.clear_window());
            std::mem::drop(proxy.reload_settings());
            std::mem::drop(proxy.reload_widget_settings());
            std::mem::drop(proxy.toggle_manual_stopwatch());
            std::mem::drop(proxy.reset_manual_stopwatch());
            std::mem::drop(proxy.shortcut_availability());
        }

        let _ = assert_methods;
    }

    #[test]
    fn report_window_uses_signed_transport_integers_for_kwin_javascript() {
        fn assert_signed_transport(proxy: &Core1Proxy<'_>) {
            std::mem::drop(proxy.report_window(-1, "invalid", "invalid", 0, 0, -1, -1, "1.25"));
        }

        let _ = assert_signed_transport;
    }
}
