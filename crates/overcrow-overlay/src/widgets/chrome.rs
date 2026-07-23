//! Shared overlay chrome: frames, resize grip, Warframe accent colors.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2, vec2};
use overcrow_config::{WIDGET_PANEL_MAX, WIDGET_PANEL_MIN, WIDGET_PANEL_MIN_HEIGHT};

pub const TITLE_SIZE: f32 = 13.0;
pub const BODY_SIZE: f32 = 15.0;
pub const META_SIZE: f32 = 12.0;
pub const TIMER_SIZE: f32 = 16.0;

/// Shared panel chrome for resizable / Warframe widgets.
pub fn panel_frame(transparent_background: bool) -> egui::Frame {
    styled_panel_frame(
        transparent_background,
        Color32::from_black_alpha(220),
        Color32::from_white_alpha(42),
        10,
        egui::Margin::symmetric(16, 12),
    )
}

/// Compact chrome for session / clock / performance / stopwatch / media.
pub fn compact_panel_frame(transparent_background: bool) -> egui::Frame {
    styled_panel_frame(
        transparent_background,
        Color32::from_black_alpha(210),
        Color32::from_white_alpha(36),
        8,
        egui::Margin::symmetric(18, 12),
    )
}

fn styled_panel_frame(
    transparent_background: bool,
    fill: Color32,
    stroke_color: Color32,
    corner_radius: u8,
    inner_margin: egui::Margin,
) -> egui::Frame {
    if transparent_background {
        egui::Frame::new()
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(corner_radius)
            .inner_margin(inner_margin)
    } else {
        egui::Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0, stroke_color))
            .corner_radius(corner_radius)
            .inner_margin(inner_margin)
    }
}

pub fn title_text(label: &str) -> egui::RichText {
    egui::RichText::new(label)
        .size(TITLE_SIZE)
        .strong()
        .color(Color32::from_gray(200))
}

pub fn meta_text(label: impl Into<String>) -> egui::RichText {
    egui::RichText::new(label)
        .size(META_SIZE)
        .color(Color32::from_gray(165))
}

/// Gear icon that opens a compact options submenu (filters stay outside).
pub fn options_menu(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let icon = egui::RichText::new("⚙")
        .size(META_SIZE + 2.0)
        .color(Color32::from_gray(175));
    ui.menu_button(icon, |ui| {
        ui.set_min_width(168.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        add_contents(ui);
    });
}

pub fn cycle_state_color(state: &str) -> Color32 {
    match state {
        "jour" | "day" => Color32::from_rgb(255, 210, 90),
        "nuit" | "night" => Color32::from_rgb(120, 170, 255),
        "chaud" | "warm" => Color32::from_rgb(255, 140, 90),
        "froid" | "cold" => Color32::from_rgb(140, 210, 255),
        "fass" => Color32::from_rgb(255, 120, 90),
        "vome" => Color32::from_rgb(170, 120, 255),
        "corpus" => Color32::from_rgb(90, 190, 255),
        "grineer" => Color32::from_rgb(120, 200, 110),
        _ => Color32::from_gray(210),
    }
}

pub fn era_color(era: &str) -> Color32 {
    match era {
        "Lith" => Color32::from_rgb(200, 200, 210),
        "Meso" => Color32::from_rgb(130, 200, 255),
        "Neo" => Color32::from_rgb(150, 230, 160),
        "Axi" => Color32::from_rgb(255, 200, 120),
        "Requiem" => Color32::from_rgb(220, 140, 255),
        "Omni" => Color32::from_rgb(255, 150, 180),
        _ => Color32::from_gray(200),
    }
}

pub fn timer_color() -> Color32 {
    Color32::from_rgb(230, 230, 235)
}

pub fn accent_ok() -> Color32 {
    Color32::from_rgb(140, 220, 150)
}

pub fn accent_warn() -> Color32 {
    Color32::from_rgb(255, 190, 100)
}

pub fn accent_error() -> Color32 {
    Color32::from_rgb(255, 140, 140)
}

/// Clamp a panel size into allowed bounds.
pub fn clamp_panel_size(size: Vec2) -> Vec2 {
    clamp_panel_size_min(size, WIDGET_PANEL_MIN)
}

/// Like [`clamp_panel_size`], with a custom minimum width (e.g. fissures).
pub fn clamp_panel_size_min(size: Vec2, min_width: f32) -> Vec2 {
    vec2(
        size.x.clamp(min_width, WIDGET_PANEL_MAX),
        size.y.clamp(WIDGET_PANEL_MIN_HEIGHT, WIDGET_PANEL_MAX),
    )
}

/// Fissure panels can shrink a bit narrower than the global default.
pub const FISSURE_PANEL_MIN_WIDTH: f32 = 250.0;

const GRIP_PX: f32 = 18.0;

/// Bottom-right resize grip.
///
/// Returns pointer drag delta this frame. Callers own size state and keep the
/// panel top-left fixed for the gesture.
pub fn resize_grip(ui: &mut egui::Ui, panel_rect: Rect, enabled: bool) -> ResizeGripOutcome {
    if !enabled {
        return ResizeGripOutcome::default();
    }

    let grip_rect = Rect::from_min_size(
        panel_rect.max - vec2(GRIP_PX, GRIP_PX),
        vec2(GRIP_PX, GRIP_PX),
    );
    let response = ui.interact(
        grip_rect,
        ui.id().with("widget-resize-grip"),
        Sense::click_and_drag(),
    );

    let stroke = if response.hovered() || response.dragged() {
        Stroke::new(2.0, Color32::from_white_alpha(180))
    } else {
        Stroke::new(1.5, Color32::from_white_alpha(90))
    };
    let painter = ui.painter();
    for offset in [0.0_f32, 5.0, 10.0] {
        let a = Pos2::new(grip_rect.max.x - 3.0 - offset, grip_rect.max.y - 3.0);
        let b = Pos2::new(grip_rect.max.x - 3.0, grip_rect.max.y - 3.0 - offset);
        painter.line_segment([a, b], stroke);
    }

    ResizeGripOutcome {
        drag_delta: if response.dragged() {
            response.drag_delta()
        } else {
            Vec2::ZERO
        },
        dragging: response.dragged(),
        drag_stopped: response.drag_stopped(),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ResizeGripOutcome {
    pub drag_delta: Vec2,
    pub dragging: bool,
    pub drag_stopped: bool,
}

/// Fixed width from the user profile. Height is capped separately by the caller.
pub fn panel_width_limits(ui: &mut egui::Ui, width: f32) {
    ui.set_min_width(width);
    ui.set_max_width(width);
}

/// Report a panel size that keeps user width, never exceeds user height, and
/// never forces empty vertical padding above the content floor.
pub fn report_content_panel_size(user: Vec2, measured: Vec2) -> Vec2 {
    vec2(
        user.x,
        measured.y.min(user.y).max(WIDGET_PANEL_MIN_HEIGHT).max(1.0),
    )
}

pub fn apply_scale(ui: &mut egui::Ui, scale: f32) {
    let scale = scale.clamp(0.75, 1.75);
    if (scale - 1.0).abs() < 0.01 {
        return;
    }
    let mut style = egui::Style::clone(ui.style().as_ref());
    for font_id in style.text_styles.values_mut() {
        font_id.size = (font_id.size * scale).clamp(10.0, 42.0);
    }
    style.spacing.item_spacing.x *= scale;
    style.spacing.item_spacing.y *= scale;
    style.spacing.button_padding.x *= scale;
    style.spacing.button_padding.y *= scale;
    ui.set_style(style);
}
