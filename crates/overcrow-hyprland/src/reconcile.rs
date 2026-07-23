use crate::model::{HyprMonitor, HyprWindow, WindowAddress, WindowReport};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReconcileOutput {
    pub report: Option<WindowReport>,
}

#[derive(Debug, Default)]
pub struct Reconciler {
    last_reportable: Option<WindowAddress>,
}

impl Reconciler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reconcile(
        &mut self,
        active: Option<&HyprWindow>,
        clients: &[HyprWindow],
        monitors: &[HyprMonitor],
        preserve_game: bool,
    ) -> ReconcileOutput {
        let active_report = active
            .and_then(|active_window| {
                clients
                    .iter()
                    .find(|client| client.address == active_window.address)
            })
            .and_then(|window| WindowReport::from_window(window, monitors));

        let report = if preserve_game {
            self.remembered_report(clients, monitors)
        } else {
            match active_report {
                Some(report) if !report.is_overlay() => {
                    self.last_reportable = Some(report.address.clone());
                    Some(report)
                }
                Some(_) => self.remembered_report(clients, monitors),
                None => None,
            }
        };

        let Some(report) = report else {
            self.last_reportable = None;
            return ReconcileOutput::default();
        };

        ReconcileOutput {
            report: Some(report),
        }
    }

    fn remembered_report(
        &self,
        clients: &[HyprWindow],
        monitors: &[HyprMonitor],
    ) -> Option<WindowReport> {
        let remembered = self.last_reportable.as_ref()?;
        clients
            .iter()
            .find(|window| window.address == remembered.as_str())
            .and_then(|window| WindowReport::from_window(window, monitors))
            .filter(|candidate| !candidate.is_overlay())
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{HyprMonitor, HyprWindow, OVERLAY_APP_ID};

    use super::Reconciler;

    fn sample_window(address: &str, class: &str, monitor: i32) -> HyprWindow {
        HyprWindow {
            address: address.to_owned(),
            mapped: true,
            hidden: false,
            at: [12, 36],
            size: [2417, 1680],
            monitor,
            class: class.to_owned(),
            title: class.to_owned(),
            pid: 136_279,
            workspace: Some(crate::model::HyprWorkspace {
                id: 1,
                name: "1".to_owned(),
            }),
            tags: Vec::new(),
        }
    }

    fn monitors() -> Vec<HyprMonitor> {
        vec![HyprMonitor { id: 0, scale: 1.0 }]
    }

    #[test]
    fn reports_the_focused_game() {
        let game = sample_window("0x10", "steam_app_1623730", 0);
        let overlays = [
            sample_window("0x20", OVERLAY_APP_ID, 0),
            sample_window("0x21", OVERLAY_APP_ID, 0),
        ];
        let mut clients = vec![game.clone()];
        clients.extend(overlays);

        let output = Reconciler::new().reconcile(Some(&game), &clients, &monitors(), false);

        assert_eq!(
            output
                .report
                .as_ref()
                .expect("focused report")
                .address
                .as_str(),
            "0x10"
        );
    }

    #[test]
    fn overlay_focus_keeps_only_the_last_still_valid_window() {
        let game = sample_window("0x10", "steam_app_1623730", 0);
        let overlay = sample_window("0x20", OVERLAY_APP_ID, 0);
        let mut reconciler = Reconciler::new();
        reconciler.reconcile(
            Some(&game),
            &[game.clone(), overlay.clone()],
            &monitors(),
            false,
        );

        let retained =
            reconciler.reconcile(Some(&overlay), &[game, overlay.clone()], &monitors(), false);
        assert_eq!(
            retained.report.expect("retained report").address.as_str(),
            "0x10"
        );

        let cleared = reconciler.reconcile(
            Some(&overlay),
            std::slice::from_ref(&overlay),
            &monitors(),
            false,
        );
        assert!(cleared.report.is_none());
    }

    #[test]
    fn unrelated_focus_is_forwarded_for_core_classification() {
        let game = sample_window("0x10", "steam_app_1623730", 0);
        let browser = sample_window("0x30", "Brave-browser", 0);
        let mut reconciler = Reconciler::new();
        reconciler.reconcile(Some(&game), std::slice::from_ref(&game), &monitors(), false);

        let output =
            reconciler.reconcile(Some(&browser), &[game, browser.clone()], &monitors(), false);

        assert_eq!(
            output.report.expect("browser report").app_id,
            "Brave-browser"
        );
    }

    #[test]
    fn active_window_missing_from_clients_fails_closed() {
        let stale_active = sample_window("0x10", "steam_app_1623730", 0);
        let overlay = sample_window("0x20", OVERLAY_APP_ID, 0);

        let output = Reconciler::new().reconcile(
            Some(&stale_active),
            std::slice::from_ref(&overlay),
            &monitors(),
            false,
        );

        assert!(output.report.is_none());
    }

    #[test]
    fn missing_or_overlay_only_focus_fails_closed() {
        let overlay = sample_window("0x20", OVERLAY_APP_ID, 0);
        let mut reconciler = Reconciler::new();

        assert!(
            reconciler
                .reconcile(None, std::slice::from_ref(&overlay), &monitors(), false,)
                .report
                .is_none()
        );
        assert!(
            reconciler
                .reconcile(
                    Some(&overlay),
                    std::slice::from_ref(&overlay),
                    &monitors(),
                    false,
                )
                .report
                .is_none()
        );
    }

    #[test]
    fn interactive_unrelated_focus_retains_the_still_valid_game() {
        let game = sample_window("0x10", "steam_app_1623730", 0);
        let browser = sample_window("0x30", "Brave-browser", 0);
        let overlay = sample_window("0x20", OVERLAY_APP_ID, 0);
        let mut reconciler = Reconciler::new();
        reconciler.reconcile(
            Some(&game),
            &[game.clone(), overlay.clone()],
            &monitors(),
            false,
        );

        let output = reconciler.reconcile(
            Some(&browser),
            &[game, browser.clone(), overlay],
            &monitors(),
            true,
        );

        assert_eq!(
            output.report.expect("retained game").address.as_str(),
            "0x10"
        );
    }

    #[test]
    fn interactive_continuity_fails_closed_when_the_game_disappears() {
        let game = sample_window("0x10", "steam_app_1623730", 0);
        let browser = sample_window("0x30", "Brave-browser", 0);
        let mut reconciler = Reconciler::new();
        reconciler.reconcile(Some(&game), std::slice::from_ref(&game), &monitors(), false);

        let output = reconciler.reconcile(
            Some(&browser),
            std::slice::from_ref(&browser),
            &monitors(),
            true,
        );

        assert!(output.report.is_none());
    }
}
