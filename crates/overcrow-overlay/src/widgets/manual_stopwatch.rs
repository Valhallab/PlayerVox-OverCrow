use std::time::{Duration, Instant};

use eframe::egui;
use overcrow_config::{WidgetId, WidgetProfile};
use overcrow_protocol::{CoreSnapshot, ManualStopwatchSnapshot, OverlayMode};

use super::widget_visible;

const MANUAL_STOPWATCH_OPTIMISM: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug)]
struct ManualStopwatchAnchor {
    reported: Duration,
    elapsed: Duration,
    received_at: Instant,
    running: bool,
}

#[derive(Clone, Copy, Debug)]
enum ManualStopwatchExpectation {
    Running(bool),
    Reset {
        desired_running: bool,
        reset_acknowledged: bool,
    },
}

impl ManualStopwatchExpectation {
    fn observe(&mut self, snapshot: ManualStopwatchSnapshot) -> bool {
        match self {
            Self::Running(running) => snapshot.running == *running,
            Self::Reset {
                desired_running,
                reset_acknowledged,
            } => {
                if !*reset_acknowledged && !snapshot.running && snapshot.elapsed_ms == 0 {
                    *reset_acknowledged = true;
                    !*desired_running
                } else {
                    *reset_acknowledged && snapshot.running == *desired_running
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ManualStopwatchOptimism {
    expected: ManualStopwatchExpectation,
    optimistic_until: Instant,
}

#[derive(Debug, Default)]
pub struct ManualStopwatchClock {
    anchor: Option<ManualStopwatchAnchor>,
    /// After a local action, briefly ignore stale Core samples. Core authority
    /// wins again at the fixed deadline if the command is never acknowledged.
    optimism: Option<ManualStopwatchOptimism>,
}

impl ManualStopwatchClock {
    pub fn sync(&mut self, snapshot: ManualStopwatchSnapshot, received_at: Instant) {
        if let Some(mut optimism) = self.optimism {
            if optimism.expected.observe(snapshot) {
                self.optimism = None;
            } else if received_at < optimism.optimistic_until {
                // Stale Core sample while our optimistic action is in flight.
                self.optimism = Some(optimism);
                return;
            } else {
                self.optimism = None;
            }
        }

        let reported = Duration::from_millis(snapshot.elapsed_ms);
        let elapsed = self.anchor.map_or(reported, |anchor| {
            if anchor.running && snapshot.running && reported >= anchor.reported {
                reported.max(self.elapsed_at(received_at))
            } else {
                reported
            }
        });
        self.anchor = Some(ManualStopwatchAnchor {
            reported,
            elapsed,
            received_at,
            running: snapshot.running,
        });
    }

    /// Freeze or resume immediately on UI toggle so the display does not keep
    /// interpolating while the Core command is in flight.
    pub fn apply_local_toggle(&mut self, now: Instant) {
        let elapsed = self.elapsed_at(now);
        let running = !self.running();
        self.anchor = Some(ManualStopwatchAnchor {
            reported: elapsed,
            elapsed,
            received_at: now,
            running,
        });
        if let Some(ManualStopwatchOptimism {
            expected:
                ManualStopwatchExpectation::Reset {
                    desired_running, ..
                },
            ..
        }) = self.optimism.as_mut()
        {
            *desired_running = running;
        } else {
            self.begin_optimism(ManualStopwatchExpectation::Running(running), now);
        }
    }

    /// Zero and pause immediately on UI reset.
    pub fn apply_local_reset(&mut self, now: Instant) {
        self.anchor = Some(ManualStopwatchAnchor {
            reported: Duration::ZERO,
            elapsed: Duration::ZERO,
            received_at: now,
            running: false,
        });
        self.begin_optimism(
            ManualStopwatchExpectation::Reset {
                desired_running: false,
                reset_acknowledged: false,
            },
            now,
        );
    }

    fn begin_optimism(&mut self, expected: ManualStopwatchExpectation, now: Instant) {
        self.optimism = Some(ManualStopwatchOptimism {
            expected,
            optimistic_until: now.checked_add(MANUAL_STOPWATCH_OPTIMISM).unwrap_or(now),
        });
    }

    pub fn elapsed_at(&self, now: Instant) -> Duration {
        self.anchor.map_or(Duration::ZERO, |anchor| {
            if anchor.running {
                anchor
                    .elapsed
                    .saturating_add(now.saturating_duration_since(anchor.received_at))
            } else {
                anchor.elapsed
            }
        })
    }

    pub fn running(&self) -> bool {
        self.anchor.is_some_and(|anchor| anchor.running)
    }

    pub fn repaint_after(&self, now: Instant) -> Option<Duration> {
        self.running().then(|| {
            // Align repaints to the next centisecond for smooth subsecond display.
            let centis = self.elapsed_at(now).subsec_millis() % 10;
            Duration::from_millis(u64::from(10 - centis))
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManualStopwatchPresentation {
    pub elapsed: String,
    pub status: &'static str,
    pub toggle_label: &'static str,
    pub controls_visible: bool,
    pub shortcut_footer: Option<(&'static str, &'static str)>,
}

impl ManualStopwatchPresentation {
    pub fn new(elapsed: Duration, running: bool, mode: OverlayMode) -> Self {
        let interaction = ManualStopwatchInteractionPolicy::new(mode);
        Self {
            elapsed: format_manual_stopwatch_elapsed(elapsed),
            status: if running { "RUNNING" } else { "PAUSED" },
            toggle_label: if running { "Pause" } else { "Start" },
            controls_visible: interaction.controls_visible,
            shortcut_footer: interaction.shortcut_footer,
        }
    }
}

pub fn format_manual_stopwatch_elapsed(elapsed: Duration) -> String {
    let total_ms = elapsed.as_millis();
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms % 3_600_000) / 60_000;
    let seconds = (total_ms % 60_000) / 1_000;
    let centis = (total_ms % 1_000) / 10;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{centis:02}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManualStopwatchAction {
    Toggle,
    Reset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ManualStopwatchInteractionPolicy {
    controls_visible: bool,
    shortcut_footer: Option<(&'static str, &'static str)>,
}

impl ManualStopwatchInteractionPolicy {
    fn new(mode: OverlayMode) -> Self {
        let controls_visible = mode == OverlayMode::Interactive;
        Self {
            controls_visible,
            shortcut_footer: controls_visible.then_some(("Super+Alt+P", "Super+Alt+R")),
        }
    }

    fn mouse_action(
        self,
        requested: Option<ManualStopwatchAction>,
    ) -> Option<ManualStopwatchAction> {
        self.controls_visible.then_some(requested).flatten()
    }
}

pub fn route_manual_stopwatch_action(
    mode: OverlayMode,
    action: Option<ManualStopwatchAction>,
    toggle: impl FnOnce(),
    reset: impl FnOnce(),
) {
    match ManualStopwatchInteractionPolicy::new(mode).mouse_action(action) {
        Some(ManualStopwatchAction::Toggle) => toggle(),
        Some(ManualStopwatchAction::Reset) => reset(),
        None => {}
    }
}

pub fn manual_stopwatch_repaint_after(
    snapshot: &CoreSnapshot,
    profile: &WidgetProfile,
    clock: &ManualStopwatchClock,
    now: Instant,
) -> Option<Duration> {
    widget_visible(
        WidgetId::ManualStopwatch,
        snapshot.overlay_mode,
        snapshot.active_game.is_some(),
        profile,
    )
    .then(|| clock.repaint_after(now))
    .flatten()
}

pub struct ManualStopwatchResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub action: Option<ManualStopwatchAction>,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_manual_stopwatch(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    elapsed: Duration,
    running: bool,
    mode: OverlayMode,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> ManualStopwatchResponse {
    let presentation = ManualStopwatchPresentation::new(elapsed, running, mode);
    let mut action = None;
    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("manual-stopwatch-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(presentation.controls_visible)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            super::chrome::compact_panel_frame(transparent_background).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("STOPWATCH")
                            .size(11.0)
                            .color(egui::Color32::from_gray(170)),
                    );
                    ui.label(
                        egui::RichText::new(presentation.status)
                            .size(11.0)
                            .color(egui::Color32::from_gray(190)),
                    );
                });
                ui.label(
                    egui::RichText::new(&presentation.elapsed)
                        .monospace()
                        .strong()
                        .size(30.0),
                );

                if presentation.controls_visible {
                    ui.horizontal(|ui| {
                        if ui.button(presentation.toggle_label).clicked() {
                            action = Some(ManualStopwatchAction::Toggle);
                        }
                        if ui.button("Reset").clicked() {
                            action = Some(ManualStopwatchAction::Reset);
                        }
                    });
                    ui.separator();
                    if let Some((toggle_shortcut, reset_shortcut)) = presentation.shortcut_footer {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(toggle_shortcut).monospace().strong());
                            ui.label("start/pause");
                            ui.separator();
                            ui.label(egui::RichText::new(reset_shortcut).monospace().strong());
                            ui.label("reset");
                        });
                    }
                }
            });
        });

    ManualStopwatchResponse {
        size: response.response.rect.size(),
        position: response.response.rect.min,
        dragged: response.response.dragged(),
        drag_stopped: response.response.drag_stopped(),
        action,
    }
}
