//! Shared PlayerVox overlay presentation primitives.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
    TextStyle, Vec2, epaint::Shadow, pos2, vec2,
};
use overcrow_config::{WIDGET_PANEL_MAX, WIDGET_PANEL_MIN, WIDGET_PANEL_MIN_HEIGHT};
use overcrow_protocol::OverlayMode;

use crate::branding::{UI_BOLD_FAMILY, UI_REGULAR_FAMILY, UI_SEMIBOLD_FAMILY};

use super::WidgetGlyph;

pub const ACCENT: Color32 = Color32::from_rgb(163, 230, 53);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(181, 241, 83);
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_unmultiplied_const(163, 230, 53, 28);
pub const BACKGROUND: Color32 = Color32::from_rgb(9, 9, 11);
pub const PANEL_FILL: Color32 = Color32::from_rgba_unmultiplied_const(17, 17, 20, 238);
pub const PANEL_FILL_COMPACT: Color32 = Color32::from_rgba_unmultiplied_const(17, 17, 20, 226);
pub const SURFACE_RAISED: Color32 = Color32::from_rgba_unmultiplied_const(30, 30, 34, 224);
pub const SURFACE_HOVER: Color32 = Color32::from_rgba_unmultiplied_const(40, 40, 45, 235);
pub const PANEL_STROKE: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 24);
pub const PANEL_STROKE_STRONG: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 42);
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(247, 247, 248);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(212, 212, 216);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(161, 161, 170);
pub const TEXT_SUBTLE: Color32 = Color32::from_rgb(113, 113, 122);
pub const DANGER: Color32 = Color32::from_rgb(251, 113, 133);
pub const WARNING: Color32 = Color32::from_rgb(251, 191, 36);

pub const BODY_SIZE: f32 = 15.0;
pub const META_SIZE: f32 = 12.0;
pub const TIMER_SIZE: f32 = 16.0;
pub const CONTROL_HEIGHT: f32 = 33.0;
const THEME_BODY_SIZE: f32 = 14.0;
const PANEL_RADIUS: u8 = 14;
const PANEL_FRAME_HORIZONTAL_MARGIN: f32 = 32.0;
const PANEL_FRAME_VERTICAL_MARGIN: f32 = 24.0;
const TOOLBAR_SIZE: Vec2 = Vec2::new(88.0, 28.0);
const TOOLBAR_GAP: f32 = 6.0;
const TOOLBAR_VIEWPORT_MARGIN: f32 = 8.0;

/// Apply the shared PlayerVox presentation once at overlay startup.
pub fn install_theme(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.animation_time = 0.12;
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(22.0, FontFamily::Name(UI_BOLD_FAMILY.into())),
        );
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(THEME_BODY_SIZE, FontFamily::Name(UI_REGULAR_FAMILY.into())),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Name(UI_SEMIBOLD_FAMILY.into())),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(META_SIZE, FontFamily::Name(UI_REGULAR_FAMILY.into())),
        );
        style.spacing.item_spacing = vec2(8.0, 7.0);
        style.spacing.button_padding = vec2(12.0, 7.0);
        style.spacing.interact_size.y = CONTROL_HEIGHT;

        let visuals = &mut style.visuals;
        visuals.dark_mode = true;
        visuals.override_text_color = Some(TEXT_PRIMARY);
        visuals.weak_text_color = Some(TEXT_MUTED);
        visuals.panel_fill = BACKGROUND;
        visuals.window_fill = PANEL_FILL;
        visuals.window_stroke = Stroke::new(1.0, PANEL_STROKE_STRONG);
        visuals.window_corner_radius = CornerRadius::same(16);
        visuals.window_shadow = Shadow {
            offset: [0, 4],
            blur: 0,
            spread: 2,
            color: Color32::from_black_alpha(80),
        };
        visuals.popup_shadow = visuals.window_shadow;
        visuals.menu_corner_radius = CornerRadius::same(11);
        visuals.extreme_bg_color = Color32::from_rgb(12, 12, 15);
        visuals.text_edit_bg_color = Some(Color32::from_rgb(15, 15, 18));
        visuals.faint_bg_color = Color32::from_white_alpha(8);
        visuals.code_bg_color = Color32::from_white_alpha(8);
        visuals.selection.bg_fill = ACCENT_SOFT;
        visuals.selection.stroke = Stroke::new(1.5, ACCENT);
        visuals.hyperlink_color = ACCENT;
        visuals.warn_fg_color = WARNING;
        visuals.error_fg_color = DANGER;

        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_fill = SURFACE_RAISED;
        widgets.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, PANEL_STROKE);
        widgets.noninteractive.corner_radius = CornerRadius::same(8);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_SECONDARY);

        widgets.inactive.bg_fill = SURFACE_RAISED;
        widgets.inactive.weak_bg_fill = SURFACE_RAISED;
        widgets.inactive.bg_stroke = Stroke::new(1.0, PANEL_STROKE);
        widgets.inactive.corner_radius = CornerRadius::same(8);
        widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_SECONDARY);

        widgets.hovered.bg_fill = SURFACE_HOVER;
        widgets.hovered.weak_bg_fill = SURFACE_HOVER;
        widgets.hovered.bg_stroke =
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(163, 230, 53, 90));
        widgets.hovered.corner_radius = CornerRadius::same(8);
        widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);

        widgets.active.bg_fill = ACCENT_SOFT;
        widgets.active.weak_bg_fill = ACCENT_SOFT;
        widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
        widgets.active.corner_radius = CornerRadius::same(8);
        widgets.active.fg_stroke = Stroke::new(1.0, ACCENT_HOVER);
        widgets.open = widgets.active;
    });
}

/// Shared panel chrome for resizable / Warframe widgets.
pub fn panel_frame(transparent_background: bool) -> egui::Frame {
    styled_panel_frame(
        transparent_background,
        PANEL_FILL,
        PANEL_STROKE,
        PANEL_RADIUS,
        egui::Margin::symmetric(16, 12),
    )
}

/// Compact chrome for session / clock / performance / stopwatch / media.
pub fn compact_panel_frame(transparent_background: bool) -> egui::Frame {
    styled_panel_frame(
        transparent_background,
        PANEL_FILL_COMPACT,
        PANEL_STROKE,
        PANEL_RADIUS,
        egui::Margin::symmetric(18, 12),
    )
}

pub fn elevated_frame(transparent_background: bool) -> egui::Frame {
    let frame = egui::Frame::new()
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(12, 10));
    if transparent_background {
        frame.fill(Color32::TRANSPARENT).stroke(Stroke::NONE)
    } else {
        frame
            .fill(SURFACE_RAISED)
            .stroke(Stroke::new(1.0, PANEL_STROKE))
    }
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
            .shadow(Shadow {
                offset: [0, 3],
                blur: 0,
                spread: 1,
                color: Color32::from_black_alpha(64),
            })
    }
}

pub fn title_text(label: &str) -> egui::RichText {
    egui::RichText::new(label)
        .text_style(TextStyle::Button)
        .strong()
        .color(TEXT_PRIMARY)
}

pub fn meta_text(label: impl Into<String>) -> egui::RichText {
    egui::RichText::new(label)
        .text_style(TextStyle::Small)
        .color(TEXT_MUTED)
}

pub fn eyebrow_text(label: impl Into<String>) -> egui::RichText {
    egui::RichText::new(label)
        .text_style(TextStyle::Button)
        .size(10.0)
        .strong()
        .color(ACCENT)
}

pub fn value_text(label: impl Into<String>, size: f32) -> egui::RichText {
    egui::RichText::new(label)
        .text_style(TextStyle::Heading)
        .size(size)
        .strong()
        .color(TEXT_PRIMARY)
}

pub fn widget_identity(ui: &mut egui::Ui, glyph: WidgetGlyph, title: &str, detail: Option<&str>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 9.0;
        paint_widget_glyph(ui, glyph, 28.0 * current_content_scale(ui), false);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(title_text(title));
            if let Some(detail) = detail {
                ui.label(meta_text(detail));
            }
        });
    });
}

pub fn status_pill(ui: &mut egui::Ui, label: &str, color: Color32) -> egui::InnerResponse<()> {
    ui.vertical(|ui| {
        egui::Frame::new()
            .fill(color.gamma_multiply(0.14))
            .stroke(Stroke::new(1.0, color.gamma_multiply(0.45)))
            .corner_radius(20)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(label)
                        .text_style(TextStyle::Button)
                        .size(10.0)
                        .strong()
                        .color(color),
                );
            })
    })
    .inner
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricContentLayout {
    Stacked,
    Inline,
}

pub fn metric_tile(ui: &mut egui::Ui, label: &str, value: &str, transparent_background: bool) {
    metric_tile_layout_sized(
        ui,
        label,
        value,
        transparent_background,
        MetricContentLayout::Stacked,
        None,
    );
}

pub fn metric_tile_layout_sized(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    transparent_background: bool,
    layout: MetricContentLayout,
    content_width: Option<f32>,
) {
    let scale = current_content_scale(ui);
    metric_tile_layout_sized_scaled(
        ui,
        label,
        value,
        transparent_background,
        layout,
        content_width,
        scale,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn metric_tile_layout_sized_scaled(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    transparent_background: bool,
    layout: MetricContentLayout,
    content_width: Option<f32>,
    scale: f32,
) {
    elevated_frame(transparent_background).show(ui, |ui| {
        ui.vertical(|ui| match layout {
            MetricContentLayout::Stacked => {
                if let Some(width) = content_width {
                    ui.set_width(width.max(1.0));
                } else {
                    ui.set_min_width(104.0);
                }
                ui.label(eyebrow_text(label).size(10.0 * scale));
                ui.label(value_text(value, (BODY_SIZE + 1.0) * scale));
            }
            MetricContentLayout::Inline => {
                if let Some(width) = content_width {
                    ui.set_width(width.max(1.0));
                } else {
                    ui.set_min_width(220.0);
                }
                ui.horizontal(|ui| {
                    ui.label(eyebrow_text(label).size(10.0 * scale));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(value_text(value, (BODY_SIZE + 1.0) * scale));
                    });
                });
            }
        });
    });
}

pub fn filter_chip(
    ui: &mut egui::Ui,
    selected: &mut bool,
    label: impl Into<egui::WidgetText>,
    accent: Color32,
) -> egui::Response {
    let fill = if *selected {
        accent.gamma_multiply(0.16)
    } else {
        Color32::from_white_alpha(8)
    };
    let stroke = if *selected {
        Stroke::new(1.0, accent.gamma_multiply(0.65))
    } else {
        Stroke::new(1.0, PANEL_STROKE)
    };
    let response = ui.add(
        egui::Button::new(label)
            .selected(*selected)
            .fill(fill)
            .stroke(stroke)
            .corner_radius(20)
            .min_size(vec2(0.0, 26.0)),
    );
    if response.clicked() {
        *selected = !*selected;
    }
    response
}

pub fn primary_button<'a>(label: impl Into<String>) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::new(label.into()).strong().color(BACKGROUND))
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(9)
        .min_size(vec2(0.0, CONTROL_HEIGHT))
}

pub fn standard_button<'a>(label: impl Into<egui::WidgetText>) -> egui::Button<'a> {
    egui::Button::new(label)
        .corner_radius(9)
        .min_size(vec2(0.0, CONTROL_HEIGHT))
}

pub fn tab_button<'a>(label: impl Into<egui::WidgetText>, selected: bool) -> egui::Button<'a> {
    standard_button(label).selected(selected)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlIcon {
    Add,
    Previous,
    Play,
    Pause,
    Next,
    Remove,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarIcon {
    PassiveVisible,
    PassiveHidden,
    Options,
    Disable,
}

pub fn widget_toolbar_rect(widget: Rect, viewport: Rect) -> Rect {
    let safe = viewport.shrink(TOOLBAR_VIEWPORT_MARGIN);
    let x = (widget.right() - TOOLBAR_SIZE.x).clamp(
        safe.left(),
        (safe.right() - TOOLBAR_SIZE.x).max(safe.left()),
    );
    let above = widget.top() - TOOLBAR_GAP - TOOLBAR_SIZE.y;
    let y = if above >= safe.top() {
        above
    } else {
        (widget.top() + TOOLBAR_GAP)
            .clamp(safe.top(), (safe.bottom() - TOOLBAR_SIZE.y).max(safe.top()))
    };
    Rect::from_min_size(pos2(x, y), TOOLBAR_SIZE)
}

pub fn widget_toolbar_hover_rect(widget: Rect, viewport: Rect) -> Rect {
    widget.union(widget_toolbar_rect(widget, viewport))
}

pub fn toolbar_icon_button(
    ui: &mut egui::Ui,
    icon: ToolbarIcon,
    accessible_label: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), accessible_label)
    });
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        8.0,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        StrokeKind::Inside,
    );
    paint_toolbar_icon(
        ui.painter(),
        rect.shrink(6.0),
        icon,
        visuals.fg_stroke.color,
    );
    response.on_hover_text(accessible_label)
}

fn paint_toolbar_icon(painter: &egui::Painter, rect: Rect, icon: ToolbarIcon, color: Color32) {
    let stroke = Stroke::new(1.7, color);
    let center = rect.center();
    match icon {
        ToolbarIcon::PassiveVisible | ToolbarIcon::PassiveHidden => {
            let arch = rect.height() * 0.32;
            let steps = 6;
            let mut outline = Vec::with_capacity((steps + 1) * 2);
            for step in 0..=steps {
                let t = step as f32 / steps as f32;
                outline.push(pos2(
                    egui::lerp(rect.x_range(), t),
                    center.y - (std::f32::consts::PI * t).sin() * arch,
                ));
            }
            for step in (0..=steps).rev() {
                let t = step as f32 / steps as f32;
                outline.push(pos2(
                    egui::lerp(rect.x_range(), t),
                    center.y + (std::f32::consts::PI * t).sin() * arch,
                ));
            }
            painter.add(Shape::closed_line(outline, stroke));
            painter.circle_filled(center, 2.2, color);
            if icon == ToolbarIcon::PassiveHidden {
                painter.line_segment(
                    [rect.left_top(), rect.right_bottom()],
                    Stroke::new(2.1, color),
                );
            }
        }
        ToolbarIcon::Options => {
            for offset in [-5.0_f32, 0.0, 5.0] {
                painter.circle_filled(center + vec2(0.0, offset), 1.8, color);
            }
        }
        ToolbarIcon::Disable => {
            painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
            painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
        }
    }
}

pub fn icon_button(ui: &mut egui::Ui, icon: ControlIcon, accessible_label: &str) -> egui::Response {
    sized_icon_button(ui, icon, accessible_label, CONTROL_HEIGHT)
}

pub fn compact_icon_button(
    ui: &mut egui::Ui,
    icon: ControlIcon,
    accessible_label: &str,
) -> egui::Response {
    sized_icon_button(ui, icon, accessible_label, 22.0)
}

fn sized_icon_button(
    ui: &mut egui::Ui,
    icon: ControlIcon,
    accessible_label: &str,
    size: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), accessible_label)
    });
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        (size * 0.27).max(5.0),
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        StrokeKind::Inside,
    );
    paint_control_icon_shape(
        ui.painter(),
        rect.shrink((size * 0.24).max(5.0)),
        icon,
        visuals.fg_stroke.color,
    );
    response.on_hover_text(accessible_label)
}

pub fn control_icon(
    ui: &mut egui::Ui,
    icon: ControlIcon,
    accessible_label: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), accessible_label)
    });
    paint_control_icon_shape(ui.painter(), rect, icon, TEXT_MUTED);
    response.on_hover_text(accessible_label)
}

fn paint_control_icon_shape(
    painter: &egui::Painter,
    rect: Rect,
    icon: ControlIcon,
    color: Color32,
) {
    let stroke = Stroke::new(1.7, color);
    let center = rect.center();
    let triangle = |points| {
        painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    };
    match icon {
        ControlIcon::Add => {
            painter.line_segment(
                [pos2(center.x, rect.top()), pos2(center.x, rect.bottom())],
                stroke,
            );
            painter.line_segment(
                [pos2(rect.left(), center.y), pos2(rect.right(), center.y)],
                stroke,
            );
        }
        ControlIcon::Previous => {
            painter.line_segment(
                [
                    pos2(rect.left(), rect.top()),
                    pos2(rect.left(), rect.bottom()),
                ],
                stroke,
            );
            triangle(vec![
                pos2(rect.left() + 2.0, center.y),
                pos2(rect.right(), rect.top()),
                pos2(rect.right(), rect.bottom()),
            ]);
        }
        ControlIcon::Play => {
            triangle(vec![
                rect.left_top(),
                pos2(rect.right(), center.y),
                rect.left_bottom(),
            ]);
        }
        ControlIcon::Pause => {
            let bar_width = (rect.width() * 0.28).max(2.0);
            painter.rect_filled(
                Rect::from_min_max(
                    rect.left_top(),
                    pos2(rect.left() + bar_width, rect.bottom()),
                ),
                1.0,
                color,
            );
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(rect.right() - bar_width, rect.top()),
                    rect.right_bottom(),
                ),
                1.0,
                color,
            );
        }
        ControlIcon::Next => {
            triangle(vec![
                pos2(rect.left(), rect.top()),
                pos2(rect.right() - 2.0, center.y),
                pos2(rect.left(), rect.bottom()),
            ]);
            painter.line_segment(
                [
                    pos2(rect.right(), rect.top()),
                    pos2(rect.right(), rect.bottom()),
                ],
                stroke,
            );
        }
        ControlIcon::Remove => {
            painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
            painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
        }
    }
}

pub fn singleline_text_edit<'a>(text: &'a mut dyn egui::TextBuffer) -> egui::TextEdit<'a> {
    // egui 0.35 does not apply `TextEdit::min_size().y` to a single-line
    // editor, so vertical padding owns the shared outer height.
    egui::TextEdit::singleline(text).margin(egui::Margin::symmetric(4, 7))
}

pub fn paint_widget_glyph(
    ui: &mut egui::Ui,
    glyph: WidgetGlyph,
    size: f32,
    emphasized: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let fill = if emphasized { ACCENT } else { ACCENT_SOFT };
    let foreground = if emphasized { BACKGROUND } else { ACCENT_HOVER };
    ui.painter().rect_filled(rect, size * 0.28, fill);
    if !emphasized {
        ui.painter().rect_stroke(
            rect,
            size * 0.28,
            Stroke::new(1.0, ACCENT.gamma_multiply(0.35)),
            StrokeKind::Inside,
        );
    }
    paint_glyph_shape(ui.painter(), rect.shrink(size * 0.23), glyph, foreground);
    response
}

fn paint_glyph_shape(painter: &egui::Painter, rect: Rect, glyph: WidgetGlyph, color: Color32) {
    let stroke = Stroke::new((rect.width() * 0.1).clamp(1.2, 2.0), color);
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.42;
    match glyph {
        WidgetGlyph::Session => {
            painter.line_segment([rect.left_top(), rect.right_top()], stroke);
            painter.line_segment([rect.left_bottom(), rect.right_bottom()], stroke);
            painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
            painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
        }
        WidgetGlyph::Clock => {
            painter.circle_stroke(center, radius, stroke);
            painter.line_segment([center, pos2(center.x, center.y - radius * 0.62)], stroke);
            painter.line_segment([center, pos2(center.x + radius * 0.5, center.y)], stroke);
        }
        WidgetGlyph::Performance => {
            for (index, factor) in [0.42_f32, 0.68, 1.0].into_iter().enumerate() {
                let x = rect.left() + rect.width() * (0.17 + index as f32 * 0.33);
                painter.line_segment(
                    [
                        pos2(x, rect.bottom()),
                        pos2(x, rect.bottom() - rect.height() * factor),
                    ],
                    stroke,
                );
            }
        }
        WidgetGlyph::Stopwatch => {
            painter.circle_stroke(center + vec2(0.0, 1.0), radius * 0.88, stroke);
            painter.line_segment(
                [
                    pos2(center.x, rect.top()),
                    pos2(center.x, rect.top() + radius * 0.35),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + vec2(0.0, 1.0),
                    center + vec2(radius * 0.45, radius * 0.35),
                ],
                stroke,
            );
        }
        WidgetGlyph::Media => {
            painter.add(Shape::convex_polygon(
                vec![
                    pos2(rect.left() + rect.width() * 0.28, rect.top()),
                    pos2(rect.right(), center.y),
                    pos2(rect.left() + rect.width() * 0.28, rect.bottom()),
                ],
                color,
                Stroke::NONE,
            ));
        }
        WidgetGlyph::Notes => {
            painter.rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
            for y in [0.32_f32, 0.54, 0.76] {
                painter.line_segment(
                    [
                        pos2(rect.left() + 3.0, rect.top() + rect.height() * y),
                        pos2(rect.right() - 3.0, rect.top() + rect.height() * y),
                    ],
                    stroke,
                );
            }
        }
        WidgetGlyph::Twitch => {
            let bubble = Rect::from_min_max(
                rect.left_top(),
                pos2(rect.right(), rect.bottom() - rect.height() * 0.2),
            );
            painter.rect_stroke(bubble, 2.0, stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    pos2(bubble.left() + rect.width() * 0.2, bubble.bottom()),
                    pos2(bubble.left() + rect.width() * 0.2, rect.bottom()),
                ],
                stroke,
            );
            for x in [0.38_f32, 0.65] {
                painter.line_segment(
                    [
                        pos2(rect.left() + rect.width() * x, rect.top() + 4.0),
                        pos2(rect.left() + rect.width() * x, rect.bottom() - 7.0),
                    ],
                    stroke,
                );
            }
        }
        WidgetGlyph::Warframe => {
            painter.add(Shape::closed_line(
                vec![
                    pos2(center.x, rect.top()),
                    pos2(rect.right(), center.y),
                    pos2(center.x, rect.bottom()),
                    pos2(rect.left(), center.y),
                ],
                stroke,
            ));
            painter.circle_filled(center, radius * 0.25, color);
        }
        WidgetGlyph::Fissures => {
            painter.circle_stroke(center, radius, stroke);
            painter.circle_stroke(center, radius * 0.52, stroke);
            painter.line_segment(
                [
                    pos2(center.x - radius * 0.25, rect.top()),
                    pos2(center.x + radius * 0.1, center.y),
                ],
                stroke,
            );
        }
        WidgetGlyph::Market => {
            painter.circle_stroke(
                center - vec2(radius * 0.16, radius * 0.16),
                radius * 0.72,
                stroke,
            );
            painter.circle_stroke(
                center + vec2(radius * 0.2, radius * 0.2),
                radius * 0.72,
                stroke,
            );
        }
        WidgetGlyph::Missions => {
            painter.circle_stroke(center, radius, stroke);
            painter.line_segment(
                [
                    pos2(rect.left() + rect.width() * 0.2, center.y),
                    pos2(center.x - 1.0, rect.bottom() - rect.height() * 0.2),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(center.x - 1.0, rect.bottom() - rect.height() * 0.2),
                    pos2(rect.right(), rect.top() + rect.height() * 0.18),
                ],
                stroke,
            );
        }
        WidgetGlyph::Invasions => {
            painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
            painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
            painter.circle_stroke(center, radius * 0.38, stroke);
        }
    }
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
    Color32::from_rgb(134, 239, 172)
}

pub fn accent_warn() -> Color32 {
    WARNING
}

pub fn accent_error() -> Color32 {
    DANGER
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

pub(super) fn resize_grip_rect(panel_rect: Rect) -> Rect {
    Rect::from_min_size(
        panel_rect.max - vec2(GRIP_PX, GRIP_PX),
        vec2(GRIP_PX, GRIP_PX),
    )
}

#[derive(Clone, Copy, Debug)]
struct ResizePointerState {
    position: Option<Pos2>,
    processed_frame: u64,
    rendered_frame: u64,
}

/// Bottom-right resize grip.
///
/// Returns pointer drag delta this frame. Callers own size state and keep the
/// panel top-left fixed for the gesture.
pub fn resize_grip(ui: &mut egui::Ui, panel_rect: Rect, enabled: bool) -> ResizeGripOutcome {
    let grip_id = ui.id().with("widget-resize-grip");
    let active_id = grip_id.with("active");
    if !enabled {
        ui.ctx()
            .data_mut(|data| data.remove::<ResizePointerState>(active_id));
        return ResizeGripOutcome::default();
    }

    let grip_rect = resize_grip_rect(panel_rect);
    // `egui` normally picks a drag target from the pointer position at the end
    // of the frame. If a compositor batches press + motion, that position may
    // already be outside this small grip. Claim the drag from the actual press
    // event, but only for the topmost Area at that position.
    let press_origin = ui.input(|input| {
        input.pointer.primary_down().then(|| {
            input.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } if grip_rect.contains(*pos) => Some(*pos),
                _ => None,
            })
        })
    });
    if press_origin
        .flatten()
        .is_some_and(|position| ui.ctx().layer_id_at(position) == Some(ui.layer_id()))
    {
        ui.ctx().set_dragged_id(grip_id);
    }
    // A resize grip has no click action. Drag-only sensing lets egui claim the
    // gesture on the press frame instead of waiting for its click threshold,
    // so the movable parent Area cannot take it over on the next frame.
    let response = ui.interact(grip_rect, grip_id, Sense::drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Resize widget")
    });

    let frame = ui.ctx().cumulative_frame_nr();
    let mut state = ui
        .ctx()
        .data(|data| data.get_temp::<ResizePointerState>(active_id))
        .unwrap_or(ResizePointerState {
            position: None,
            processed_frame: frame.wrapping_sub(1),
            rendered_frame: frame,
        });

    let missed_render = state.position.is_some()
        && state.rendered_frame != frame
        && state.rendered_frame.checked_add(1) != Some(frame);
    if missed_render {
        state.position = None;
    }
    let mut drag_delta = Vec2::ZERO;
    let mut drag_stopped = false;
    let mut drag_cancelled = missed_render;
    if state.processed_frame != frame {
        let from_global = ui.ctx().layer_transform_from_global(ui.layer_id());
        let to_local = |position| from_global.map_or(position, |transform| transform * position);
        let position = response.interact_pointer_pos();

        if !missed_render && response.drag_started() {
            state.position = ui
                .input(|input| input.pointer.press_origin())
                .map(to_local)
                .or(position);
        }
        if let Some(previous) = state.position {
            if response.dragged()
                && let Some(position) = position
            {
                drag_delta = position - previous;
                state.position = Some(position);
            } else if response.drag_stopped() {
                if let Some(position) = position {
                    drag_delta = position - previous;
                    drag_stopped = true;
                } else {
                    drag_cancelled = true;
                }
                state.position = None;
            } else {
                state.position = None;
                drag_cancelled = true;
            }
        }

        state.processed_frame = frame;
    }
    state.rendered_frame = frame;
    ui.ctx().data_mut(|data| data.insert_temp(active_id, state));
    let dragging = state.position.is_some();
    if drag_stopped {
        // Repaint once at the saved dimensions so content-driven height and
        // wrapping are measured from the final width.
        ui.ctx().request_repaint();
    }

    let stroke = if response.hovered() || dragging {
        Stroke::new(2.0, ACCENT)
    } else {
        Stroke::new(1.5, TEXT_SUBTLE)
    };
    for offset in [0.0_f32, 5.0, 10.0] {
        let a = Pos2::new(grip_rect.max.x - 3.0 - offset, grip_rect.max.y - 3.0);
        let b = Pos2::new(grip_rect.max.x - 3.0, grip_rect.max.y - 3.0 - offset);
        ui.painter().line_segment([a, b], stroke);
    }

    ResizeGripOutcome {
        drag_delta,
        dragging,
        drag_stopped,
        drag_cancelled,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ResizeGripOutcome {
    pub drag_delta: Vec2,
    pub dragging: bool,
    pub drag_stopped: bool,
    pub drag_cancelled: bool,
}

/// Fixed width from the user profile. Height is capped separately by the caller.
pub fn panel_width_limits(ui: &mut egui::Ui, width: f32, transparent_background: bool) {
    let content_width = panel_content_width(width, transparent_background);
    ui.set_min_width(content_width);
    ui.set_max_width(content_width);
}

pub fn panel_content_width(panel_width: f32, transparent_background: bool) -> f32 {
    (panel_width - panel_frame_width_budget(transparent_background)).max(1.0)
}

pub fn panel_content_height(panel_height: f32, transparent_background: bool) -> f32 {
    (panel_height - panel_frame_height_budget(transparent_background)).max(1.0)
}

fn panel_frame_width_budget(transparent_background: bool) -> f32 {
    PANEL_FRAME_HORIZONTAL_MARGIN + if transparent_background { 0.0 } else { 2.0 }
}

fn panel_frame_height_budget(transparent_background: bool) -> f32 {
    PANEL_FRAME_VERTICAL_MARGIN + if transparent_background { 0.0 } else { 2.0 }
}

/// Keep the user's full size while editing and only the chosen width in passive
/// mode. Passive height follows content up to the game viewport.
pub fn fixed_panel_constraints(
    ui: &mut egui::Ui,
    user: Vec2,
    mode: OverlayMode,
    passive_max_height: f32,
    transparent_background: bool,
) {
    let content_size = vec2(
        panel_content_width(user.x, transparent_background),
        panel_content_height(user.y, transparent_background),
    );
    if mode == OverlayMode::Interactive {
        ui.set_min_size(content_size);
        ui.set_max_size(content_size);
    } else {
        panel_width_limits(ui, user.x, transparent_background);
        ui.set_max_height(panel_content_height(
            passive_max_height,
            transparent_background,
        ));
    }
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

pub fn content_font_size(base: f32, scale: f32) -> f32 {
    base * scale.clamp(0.75, 1.75)
}

pub fn current_content_scale(ui: &egui::Ui) -> f32 {
    ui.style()
        .text_styles
        .get(&TextStyle::Body)
        .map(|font| (font.size / THEME_BODY_SIZE).clamp(0.75, 1.75))
        .unwrap_or(1.0)
}

pub fn scaled_content_font_size(ui: &egui::Ui, base: f32) -> f32 {
    content_font_size(base, current_content_scale(ui))
}
