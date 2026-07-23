use eframe::egui;
use overcrow_protocol::GameTelemetry;

const GIBIBYTE: f64 = (1024_u64 * 1024 * 1024) as f64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformancePresentation {
    pub cpu: String,
    pub ram: String,
    pub host_cpu_temperature: String,
    pub host_gpu_temperature: String,
}

impl From<Option<GameTelemetry>> for PerformancePresentation {
    fn from(telemetry: Option<GameTelemetry>) -> Self {
        let telemetry = telemetry.unwrap_or_default();

        Self {
            cpu: telemetry
                .cpu_percent_hundredths
                .map_or_else(unavailable, |hundredths| {
                    format!("{}.{:02}%", hundredths / 100, hundredths % 100)
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
}

pub fn paint_performance(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    telemetry: Option<GameTelemetry>,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> PerformanceResponse {
    let presentation = PerformancePresentation::from(telemetry);
    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("performance-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(draggable)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            super::chrome::compact_panel_frame(transparent_background).show(ui, |ui| {
                ui.label(
                    egui::RichText::new("PERFORMANCE")
                        .size(11.0)
                        .color(egui::Color32::from_gray(170)),
                );
                egui::Grid::new("performance-panel-metrics")
                    .num_columns(2)
                    .spacing(egui::vec2(18.0, 4.0))
                    .show(ui, |ui| {
                        metric_row(ui, "Game CPU", &presentation.cpu);
                        metric_row(ui, "Game RAM", &presentation.ram);
                        metric_row(ui, "Host CPU", &presentation.host_cpu_temperature);
                        metric_row(ui, "Host GPU", &presentation.host_gpu_temperature);
                    });
            });
        });

    PerformanceResponse {
        size: response.response.rect.size(),
        position: response.response.rect.min,
        dragged: response.response.dragged(),
        drag_stopped: response.response.drag_stopped(),
    }
}

fn metric_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label).color(egui::Color32::from_gray(190)));
    ui.label(egui::RichText::new(value).monospace().strong());
    ui.end_row();
}
