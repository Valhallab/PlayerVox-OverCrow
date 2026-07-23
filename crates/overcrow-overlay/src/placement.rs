use eframe::egui::{Pos2, Rect, Vec2, pos2};
use overcrow_config::WidgetPosition;

pub fn screen_position(
    viewport: Rect,
    widget_size: Vec2,
    margin: f32,
    position: WidgetPosition,
) -> Pos2 {
    pos2(
        screen_axis(
            viewport.min.x,
            viewport.width(),
            widget_size.x,
            margin,
            position.x,
        ),
        screen_axis(
            viewport.min.y,
            viewport.height(),
            widget_size.y,
            margin,
            position.y,
        ),
    )
}

pub fn normalized_position(
    viewport: Rect,
    widget_size: Vec2,
    margin: f32,
    position: Pos2,
) -> WidgetPosition {
    WidgetPosition {
        x: normalized_axis(
            viewport.min.x,
            viewport.width(),
            widget_size.x,
            margin,
            position.x,
        ),
        y: normalized_axis(
            viewport.min.y,
            viewport.height(),
            widget_size.y,
            margin,
            position.y,
        ),
    }
}

fn screen_axis(origin: f32, viewport: f32, widget: f32, margin: f32, ratio: f32) -> f32 {
    origin + margin + available_space(viewport, widget, margin) * clamped_ratio(ratio)
}

fn normalized_axis(origin: f32, viewport: f32, widget: f32, margin: f32, value: f32) -> f32 {
    let available = available_space(viewport, widget, margin);
    if available == 0.0 || !value.is_finite() {
        return 0.0;
    }
    clamped_ratio((value - origin - margin) / available)
}

fn available_space(viewport: f32, widget: f32, margin: f32) -> f32 {
    (viewport - widget - 2.0 * margin).max(0.0)
}

fn clamped_ratio(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{normalized_position, screen_position};
    use eframe::egui::{Rect, pos2, vec2};
    use overcrow_config::WidgetPosition;

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1_000.0, 800.0))
    }

    #[test]
    fn maps_normalized_corners_into_the_widget_safe_area() {
        let widget = vec2(200.0, 100.0);

        assert_eq!(
            screen_position(viewport(), widget, 24.0, WidgetPosition { x: 0.0, y: 0.0 }),
            pos2(24.0, 24.0)
        );
        assert_eq!(
            screen_position(viewport(), widget, 24.0, WidgetPosition { x: 1.0, y: 1.0 }),
            pos2(776.0, 676.0)
        );
    }

    #[test]
    fn maps_midpoint_relative_to_a_non_zero_viewport_origin() {
        let viewport = Rect::from_min_size(pos2(100.0, 200.0), vec2(1_000.0, 800.0));

        assert_eq!(
            screen_position(
                viewport,
                vec2(200.0, 100.0),
                24.0,
                WidgetPosition { x: 0.5, y: 0.5 }
            ),
            pos2(500.0, 550.0)
        );
    }

    #[test]
    fn clamps_ratios_before_mapping() {
        assert_eq!(
            screen_position(
                viewport(),
                vec2(200.0, 100.0),
                24.0,
                WidgetPosition { x: -2.0, y: 3.0 }
            ),
            pos2(24.0, 676.0)
        );
    }

    #[test]
    fn zero_available_space_has_a_stable_origin_and_ratio() {
        let small = Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 80.0));
        let widget = vec2(120.0, 90.0);
        let position = screen_position(small, widget, 24.0, WidgetPosition { x: 1.0, y: 1.0 });

        assert_eq!(position, pos2(34.0, 44.0));
        assert_eq!(
            normalized_position(small, widget, 24.0, position),
            WidgetPosition::default()
        );
    }

    #[test]
    fn inverse_mapping_round_trips() {
        let normalized = WidgetPosition { x: 0.25, y: 0.75 };
        let widget = vec2(200.0, 100.0);
        let position = screen_position(viewport(), widget, 24.0, normalized);

        assert_eq!(
            normalized_position(viewport(), widget, 24.0, position),
            normalized
        );
    }

    #[test]
    fn inverse_mapping_clamps_positions_outside_the_safe_area() {
        assert_eq!(
            normalized_position(viewport(), vec2(200.0, 100.0), 24.0, pos2(-100.0, 2_000.0)),
            WidgetPosition { x: 0.0, y: 1.0 }
        );
    }
}
