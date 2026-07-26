use std::{num::NonZeroUsize, sync::LazyLock};

use eframe::egui;
use overcrow_config::PerformanceLayout;
use overcrow_protocol::GameTelemetry;

use super::chrome::{
    MetricContentLayout, ResizeGripOutcome, apply_scale, metric_tile_layout_sized_scaled,
    resize_grip,
};

const GIBIBYTE: f64 = (1024_u64 * 1024 * 1024) as f64;
const PANEL_HORIZONTAL_MARGIN: f32 = 36.0;
const TILE_HORIZONTAL_MARGIN: f32 = 24.0;
const FOUR_COLUMN_MIN_SAFE_WIDTH: f32 = 560.0;
const PERFORMANCE_WIDTH_DEFAULT: f32 = 580.0;
const PERFORMANCE_WIDTH_MIN: f32 = 300.0;
const PERFORMANCE_WIDTH_MAX: f32 = 900.0;
static LOGICAL_PROCESSORS: LazyLock<NonZeroUsize> =
    LazyLock::new(|| std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN));

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformancePresentation {
    pub cpu: String,
    pub ram: String,
    pub host_cpu_temperature: String,
    pub host_gpu_temperature: String,
}

impl PerformancePresentation {
    pub fn new(telemetry: Option<GameTelemetry>, logical_processors: NonZeroUsize) -> Self {
        let telemetry = telemetry.unwrap_or_default();

        Self {
            cpu: telemetry
                .cpu_percent_hundredths
                .map_or_else(unavailable, |hundredths| {
                    format_normalized_cpu(hundredths, logical_processors)
                }),
            ram: telemetry.resident_bytes.map_or_else(unavailable, |bytes| {
                format!("{:.2} GiB", bytes as f64 / GIBIBYTE)
            }),
            host_cpu_temperature: telemetry
                .cpu_temperature_millicelsius
                .map_or_else(unavailable, format_temperature),
            host_gpu_temperature: telemetry
                .gpu_temperature_millicelsius
                .map_or_else(unavailable, format_temperature),
        }
    }
}

fn format_normalized_cpu(hundredths: u32, logical_processors: NonZeroUsize) -> String {
    let processors = logical_processors.get() as u128;
    let normalized = (u128::from(hundredths) + processors / 2) / processors;
    let normalized = normalized.min(10_000) as u32;
    format!("{}.{:02}%", normalized / 100, normalized % 100)
}

fn format_temperature(millicelsius: i64) -> String {
    format!("{:.1} °C", millicelsius as f64 / 1_000.0)
}

fn unavailable() -> String {
    "—".to_owned()
}

pub struct PerformanceResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub resize: ResizeGripOutcome,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_performance(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    telemetry: Option<GameTelemetry>,
    layout: PerformanceLayout,
    scale: f32,
    preferred_width: f32,
    transparent_background: bool,
    draggable: bool,
    interactive: bool,
    margin: f32,
) -> PerformanceResponse {
    let presentation = PerformancePresentation::new(telemetry, *LOGICAL_PROCESSORS);
    let metrics = [
        ("GAME CPU", presentation.cpu.as_str()),
        ("GAME RAM", presentation.ram.as_str()),
        ("CPU TEMP", presentation.host_cpu_temperature.as_str()),
        ("GPU TEMP", presentation.host_gpu_temperature.as_str()),
    ];
    let viewport = ui.max_rect();
    let safe_width = (viewport.width() - margin * 2.0).max(1.0);
    let panel_width = performance_panel_width(layout, preferred_width, safe_width);
    let mut resize = ResizeGripOutcome::default();
    let response = egui::Area::new(egui::Id::new("performance-panel"))
        .current_pos(current_position)
        .movable(draggable)
        // Keep the resize grip interactive while panel movement is suppressed
        // near the bottom-right corner.
        .interactable(interactive)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            apply_scale(ui, scale);
            let frame = super::chrome::compact_panel_frame(transparent_background).show(ui, |ui| {
                let inner_width =
                    (panel_width - frame_width_budget(transparent_background)).max(1.0);
                let tile_horizontal_budget =
                    frame_budget(TILE_HORIZONTAL_MARGIN, transparent_background);
                ui.set_width(inner_width);
                let paint_metric =
                    |ui: &mut egui::Ui, label: &str, value: &str, content_width: f32| {
                        metric_tile_layout_sized_scaled(
                            ui,
                            label,
                            value,
                            transparent_background,
                            match layout {
                                PerformanceLayout::Horizontal => MetricContentLayout::Stacked,
                                PerformanceLayout::Vertical => MetricContentLayout::Inline,
                            },
                            Some(content_width),
                            scale,
                        );
                    };
                match layout {
                    PerformanceLayout::Horizontal => {
                        let columns = performance_columns(panel_width);
                        let gaps = ui.spacing().item_spacing.x * (columns - 1) as f32;
                        let tile_width = ((inner_width - gaps) / columns as f32).max(1.0);
                        let content_width = (tile_width - tile_horizontal_budget).max(1.0);
                        for row in metrics.chunks(columns) {
                            ui.horizontal(|ui| {
                                for (label, value) in row {
                                    paint_metric(ui, label, value, content_width);
                                }
                            });
                        }
                    }
                    PerformanceLayout::Vertical => {
                        ui.vertical(|ui| {
                            let content_width =
                                (inner_width - tile_horizontal_budget).clamp(1.0, 220.0);
                            for (label, value) in metrics {
                                paint_metric(ui, label, value, content_width);
                            }
                        });
                    }
                }
            });
            resize = resize_grip(
                ui,
                frame.response.rect,
                interactive && layout == PerformanceLayout::Horizontal,
            );
        });

    PerformanceResponse {
        size: response.response.rect.size(),
        position: response.response.rect.min,
        dragged: response.response.dragged(),
        drag_stopped: response.response.drag_stopped(),
        resize,
    }
}

fn frame_width_budget(transparent_background: bool) -> f32 {
    frame_budget(PANEL_HORIZONTAL_MARGIN, transparent_background)
}

fn frame_budget(margin: f32, transparent_background: bool) -> f32 {
    margin + if transparent_background { 0.0 } else { 2.0 }
}

fn performance_panel_width(
    layout: PerformanceLayout,
    preferred_width: f32,
    safe_width: f32,
) -> f32 {
    let requested = match layout {
        PerformanceLayout::Horizontal if preferred_width > 0.0 => preferred_width,
        PerformanceLayout::Horizontal => PERFORMANCE_WIDTH_DEFAULT,
        PerformanceLayout::Vertical => PERFORMANCE_WIDTH_MIN,
    };
    let maximum = PERFORMANCE_WIDTH_MAX.min(safe_width).max(1.0);
    requested.clamp(PERFORMANCE_WIDTH_MIN.min(maximum), maximum)
}

pub(super) fn performance_columns(panel_width: f32) -> usize {
    if panel_width >= FOUR_COLUMN_MIN_SAFE_WIDTH {
        4
    } else {
        2
    }
}
