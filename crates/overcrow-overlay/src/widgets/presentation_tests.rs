use std::{cell::Cell, num::NonZeroUsize, time::Duration};

use chrono::{Local, TimeZone, Timelike};
use eframe::egui::{self, RawInput, Rect, pos2, vec2};
use overcrow_config::PerformanceLayout;
use overcrow_protocol::{GameTelemetry, OverlayMode};

use crate::branding::install_fonts;
use crate::media::MediaSnapshot;

use super::{
    ClockPresentation, PerformancePresentation,
    chrome::{
        ACCENT, BACKGROUND, CONTROL_HEIGHT, ControlIcon, PANEL_FILL, PANEL_FILL_COMPACT,
        PANEL_STROKE, ResizeGripOutcome, SURFACE_RAISED, TEXT_MUTED, TEXT_PRIMARY, ToolbarIcon,
        compact_panel_frame, content_font_size, elevated_frame, icon_button, install_theme,
        panel_frame, primary_button, resize_grip, singleline_text_edit, standard_button,
        status_pill, tab_button, toolbar_icon_button, widget_toolbar_hover_rect,
        widget_toolbar_rect,
    },
};

fn paint_resize_areas<const N: usize>(
    context: &egui::Context,
    viewport: Rect,
    panel: Rect,
    ids: [&'static str; N],
    events: Vec<egui::Event>,
) -> [ResizeGripOutcome; N] {
    let mut outcomes = [ResizeGripOutcome::default(); N];
    let _ = context.run_ui(
        RawInput {
            screen_rect: Some(viewport),
            events,
            ..RawInput::default()
        },
        |ui| {
            for (index, id) in ids.into_iter().enumerate() {
                outcomes[index] = egui::Area::new(egui::Id::new(id))
                    .fixed_pos(panel.min)
                    .show(ui.ctx(), |ui| {
                        ui.set_min_size(panel.size());
                        resize_grip(ui, panel, true)
                    })
                    .inner;
            }
        },
    );
    outcomes
}

#[test]
fn explicit_content_typography_uses_the_widget_scale() {
    assert_eq!(content_font_size(14.0, 0.75), 10.5);
    assert_eq!(content_font_size(14.0, 1.0), 14.0);
    assert_eq!(content_font_size(14.0, 1.75), 24.5);
}

#[test]
fn widget_palette_matches_the_playervox_dark_surface_contract() {
    assert_eq!(ACCENT, eframe::egui::Color32::from_rgb(163, 230, 53));
    assert!(PANEL_FILL.a() >= 210);
    assert!(PANEL_STROKE.a() < PANEL_FILL.a());
    assert!(TEXT_PRIMARY.r() > TEXT_MUTED.r());
}

#[test]
fn redesigned_frames_preserve_transparent_background_semantics() {
    for frame in [panel_frame(true), compact_panel_frame(true)] {
        assert_eq!(frame.fill, egui::Color32::TRANSPARENT);
        assert_eq!(frame.stroke, egui::Stroke::NONE);
    }
    for frame in [panel_frame(false), compact_panel_frame(false)] {
        assert!(frame.fill.a() > 0);
        assert!(frame.stroke.width > 0.0);
    }

    let transparent_content = elevated_frame(true);
    assert_eq!(transparent_content.fill, egui::Color32::TRANSPARENT);
    assert_eq!(transparent_content.stroke, egui::Stroke::NONE);
    let opaque_content = elevated_frame(false);
    assert!(opaque_content.fill.a() > 0);
    assert!(opaque_content.stroke.width > 0.0);
}

#[test]
fn only_the_topmost_overlapping_widget_captures_a_resize_press() {
    let context = egui::Context::default();
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    let panel = Rect::from_min_size(pos2(40.0, 40.0), vec2(200.0, 140.0));
    let grip = panel.max - vec2(8.0, 8.0);

    paint_resize_areas(
        &context,
        viewport,
        panel,
        ["back-panel", "front-panel"],
        Vec::new(),
    );
    paint_resize_areas(
        &context,
        viewport,
        panel,
        ["back-panel", "front-panel"],
        Vec::new(),
    );
    let outcomes = paint_resize_areas(
        &context,
        viewport,
        panel,
        ["back-panel", "front-panel"],
        vec![
            egui::Event::PointerMoved(grip),
            egui::Event::PointerButton {
                pos: grip,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerMoved(grip + vec2(30.0, 20.0)),
        ],
    );

    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.dragging).count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .find(|outcome| outcome.dragging)
            .map(|outcome| outcome.drag_delta),
        Some(vec2(30.0, 20.0))
    );
}

#[test]
fn a_press_and_release_in_one_frame_cannot_leave_the_grip_dragged() {
    let context = egui::Context::default();
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    let panel = Rect::from_min_size(pos2(40.0, 40.0), vec2(200.0, 140.0));
    let grip = panel.max - vec2(8.0, 8.0);

    for _ in 0..2 {
        paint_resize_areas(&context, viewport, panel, ["resizable-panel"], Vec::new());
    }
    let [outcome] = paint_resize_areas(
        &context,
        viewport,
        panel,
        ["resizable-panel"],
        vec![
            egui::Event::PointerMoved(grip),
            egui::Event::PointerButton {
                pos: grip,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: grip,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ],
    );

    assert!(!outcome.dragging);
    assert!(context.dragged_id().is_none());
    let [next] = paint_resize_areas(
        &context,
        viewport,
        panel,
        ["resizable-panel"],
        vec![egui::Event::PointerMoved(grip + vec2(40.0, 30.0))],
    );
    assert!(!next.dragging);
}

#[test]
fn resize_capture_does_not_resume_after_the_widget_misses_a_frame() {
    let context = egui::Context::default();
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    let panel = Rect::from_min_size(pos2(40.0, 40.0), vec2(200.0, 140.0));
    let grip = panel.max - vec2(8.0, 8.0);

    for _ in 0..2 {
        paint_resize_areas(&context, viewport, panel, ["resizable-panel"], Vec::new());
    }
    let [pressed] = paint_resize_areas(
        &context,
        viewport,
        panel,
        ["resizable-panel"],
        vec![
            egui::Event::PointerMoved(grip),
            egui::Event::PointerButton {
                pos: grip,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
    );
    assert!(pressed.dragging);

    paint_resize_areas(&context, viewport, panel, [], Vec::new());
    let [resumed] = paint_resize_areas(
        &context,
        viewport,
        panel,
        ["resizable-panel"],
        vec![egui::Event::PointerMoved(grip + vec2(40.0, 30.0))],
    );

    assert!(resumed.drag_cancelled);
    assert!(!resumed.dragging);
    assert_eq!(resumed.drag_delta, egui::Vec2::ZERO);
}

#[test]
fn primary_buttons_render_dark_text_on_the_lime_surface() {
    let context = egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    let output = context.run_ui(RawInput::default(), |ui| {
        ui.add(primary_button("Action"));
    });

    let text = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Action" => Some(text),
            _ => None,
        })
        .next()
        .expect("the primary button must paint its label");
    let colors = text
        .galley
        .rows
        .iter()
        .flat_map(|row| {
            row.visuals.mesh.vertices[row.visuals.glyph_vertex_range.clone()]
                .iter()
                .map(|vertex| vertex.color)
        })
        .collect::<Vec<_>>();
    assert!(!colors.is_empty());
    assert!(
        colors.iter().all(|color| *color == BACKGROUND),
        "{colors:?}"
    );
}

#[test]
fn status_pills_keep_their_natural_height_in_a_tall_header_row() {
    let context = egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    let height = Cell::new(0.0);
    let _ = context.run_ui(RawInput::default(), |ui| {
        ui.horizontal(|ui| {
            ui.allocate_space(vec2(200.0, 90.0));
            let response = status_pill(ui, "2 ACTIVE", ACCENT);
            height.set(response.response.rect.height());
        });
    });
    assert!(height.get() <= 32.0, "pill height was {}", height.get());
}

#[test]
fn widget_toolbar_floats_above_the_top_right_when_space_allows() {
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let widget = Rect::from_min_size(pos2(120.0, 160.0), vec2(360.0, 240.0));

    let toolbar = widget_toolbar_rect(widget, viewport);

    assert!(toolbar.bottom() < widget.top());
    assert_eq!(toolbar.right(), widget.right());
    assert_eq!(toolbar.size(), vec2(88.0, 28.0));
    assert!(viewport.contains_rect(toolbar));
}

#[test]
fn widget_toolbar_folds_inside_near_the_top_edge() {
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let widget = Rect::from_min_size(pos2(120.0, 4.0), vec2(360.0, 240.0));

    let toolbar = widget_toolbar_rect(widget, viewport);

    assert!(toolbar.top() >= widget.top());
    assert!(toolbar.bottom() <= widget.bottom());
    assert!(viewport.contains_rect(toolbar));
}

#[test]
fn widget_toolbar_hover_region_bridges_the_floating_gap() {
    let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let widget = Rect::from_min_size(pos2(120.0, 160.0), vec2(360.0, 240.0));
    let toolbar = widget_toolbar_rect(widget, viewport);
    let bridge = pos2(toolbar.center().x, (toolbar.bottom() + widget.top()) * 0.5);

    assert!(widget_toolbar_hover_rect(widget, viewport).contains(bridge));
}

#[test]
fn passive_toolbar_control_uses_a_curved_eye_outline() {
    let context = egui::Context::default();
    let output = context.run_ui(RawInput::default(), |ui| {
        toolbar_icon_button(ui, ToolbarIcon::PassiveVisible, "Hide in passive mode");
    });
    let longest_path = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Path(path) => Some(path.points.len()),
            _ => None,
        })
        .max()
        .unwrap_or_default();

    assert!(
        longest_path >= 10,
        "eye outline is still a polygonal placeholder: {longest_path} points"
    );
}

#[test]
fn shared_single_line_controls_use_one_height() {
    let context = egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    let heights = std::cell::RefCell::new(Vec::new());
    let mut value = String::new();
    let _ = context.run_ui(RawInput::default(), |ui| {
        ui.horizontal(|ui| {
            heights
                .borrow_mut()
                .push(ui.add(standard_button("Action")).rect.height());
            heights
                .borrow_mut()
                .push(ui.add(primary_button("Primary")).rect.height());
            heights
                .borrow_mut()
                .push(icon_button(ui, ControlIcon::Play, "Play").rect.height());
            heights
                .borrow_mut()
                .push(ui.add(tab_button("Tab", false)).rect.height());
            heights
                .borrow_mut()
                .push(ui.add(singleline_text_edit(&mut value)).rect.height());
        });
    });

    let heights = heights.into_inner();
    assert_eq!(heights.len(), 5);
    for height in &heights {
        assert!(
            (*height - CONTROL_HEIGHT).abs() <= 0.5,
            "control heights were {heights:?}"
        );
    }
}

#[test]
fn shared_icon_buttons_are_square() {
    let context = egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    let size = Cell::new(egui::Vec2::ZERO);
    let _ = context.run_ui(RawInput::default(), |ui| {
        size.set(
            icon_button(ui, ControlIcon::Pause, "Play or pause")
                .rect
                .size(),
        );
    });

    assert!(
        (size.get().x - size.get().y).abs() <= 0.5,
        "{:?}",
        size.get()
    );
    assert!((size.get().y - CONTROL_HEIGHT).abs() <= 0.5);
}

#[test]
fn media_width_tracks_content_between_compact_bounds() {
    let context = egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    let render = |snapshot: &MediaSnapshot| {
        let mut size = egui::Vec2::ZERO;
        let _ = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0))),
                ..RawInput::default()
            },
            |ui| {
                size = super::media::paint_media(
                    ui,
                    pos2(24.0, 24.0),
                    snapshot,
                    OverlayMode::Passive,
                    1.0,
                    false,
                    false,
                    true,
                    24.0,
                )
                .size;
            },
        );
        size
    };
    let short = render(&MediaSnapshot {
        bus_name: Some("test".to_owned()),
        title: Some("Track".to_owned()),
        artist: Some("Artist".to_owned()),
        ..MediaSnapshot::default()
    });
    let long = render(&MediaSnapshot {
        bus_name: Some("test".to_owned()),
        title: Some(
            "A considerably longer track title that should expand the media widget".to_owned(),
        ),
        artist: Some("A longer artist name".to_owned()),
        ..MediaSnapshot::default()
    });

    assert!(short.x >= 240.0 && short.x <= 560.0, "{short:?}");
    assert!(long.x > short.x, "{short:?} {long:?}");
    assert!(long.x <= 560.5, "{long:?}");
}

#[test]
fn redesigned_compact_widgets_keep_natural_bounded_sizes() {
    let context = egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    let mut sizes = Vec::new();
    let _ = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0))),
            ..RawInput::default()
        },
        |ui| {
            sizes.push(
                super::session::paint_session(
                    ui,
                    pos2(24.0, 24.0),
                    Some(Duration::from_secs(90)),
                    1.0,
                    false,
                    false,
                    24.0,
                )
                .size,
            );
            sizes.push(
                super::clock::paint_clock(ui, pos2(420.0, 24.0), true, 1.0, false, false, 24.0)
                    .size,
            );
            sizes.push(
                super::performance::paint_performance(
                    ui,
                    pos2(24.0, 220.0),
                    Some(GameTelemetry::default()),
                    PerformanceLayout::Horizontal,
                    1.0,
                    0.0,
                    false,
                    false,
                    false,
                    24.0,
                )
                .size,
            );
        },
    );

    assert_eq!(sizes.len(), 3);
    for size in sizes {
        assert!(size.x > 100.0 && size.x < 600.0, "{size:?}");
        assert!(size.y > 40.0 && size.y < 300.0, "{size:?}");
    }
}

#[test]
fn compact_widget_scale_changes_content_without_scaling_shared_controls() {
    let render = |scale| {
        let context = egui::Context::default();
        install_fonts(&context);
        install_theme(&context);
        let mut size = egui::Vec2::ZERO;
        let _ = context.run_ui(RawInput::default(), |ui| {
            size = super::session::paint_session(
                ui,
                pos2(24.0, 24.0),
                Some(Duration::from_secs(90)),
                scale,
                false,
                false,
                24.0,
            )
            .size;
        });
        size
    };

    let compact = render(0.75);
    let large = render(1.75);
    assert!(large.x > compact.x, "{compact:?} {large:?}");
    assert!(large.y > compact.y, "{compact:?} {large:?}");
}

#[test]
fn transparent_performance_widget_omits_dark_panel_surfaces() {
    let context = egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    let output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0))),
            ..RawInput::default()
        },
        |ui| {
            super::performance::paint_performance(
                ui,
                pos2(24.0, 24.0),
                Some(GameTelemetry::default()),
                PerformanceLayout::Horizontal,
                1.0,
                0.0,
                true,
                false,
                false,
                24.0,
            );
        },
    );

    let fills = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) => Some(rect.fill),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!fills.contains(&PANEL_FILL));
    assert!(!fills.contains(&PANEL_FILL_COMPACT));
    assert!(!fills.contains(&SURFACE_RAISED));
}

fn telemetry(cpu_percent_hundredths: u32, resident_bytes: u64) -> GameTelemetry {
    GameTelemetry {
        cpu_percent_hundredths: Some(cpu_percent_hundredths),
        resident_bytes: Some(resident_bytes),
        ..GameTelemetry::default()
    }
}

#[test]
fn performance_uses_explicit_unavailable_markers() {
    let view = PerformancePresentation::new(None, NonZeroUsize::MIN);

    assert_eq!(view.cpu, "—");
    assert_eq!(view.ram, "—");
    assert_eq!(view.host_cpu_temperature, "—");
    assert_eq!(view.host_gpu_temperature, "—");
}

#[test]
fn cpu_hundredths_and_binary_ram_are_formatted_deterministically() {
    let view = PerformancePresentation::new(
        Some(telemetry(12_345, 3 * 1024 * 1024 * 1024)),
        NonZeroUsize::new(8).unwrap(),
    );

    assert_eq!(view.cpu, "15.43%");
    assert_eq!(view.ram, "3.00 GiB");
}

#[test]
fn normalized_game_cpu_is_bounded_to_total_logical_capacity() {
    let view =
        PerformancePresentation::new(Some(telemetry(2_000_000, 0)), NonZeroUsize::new(8).unwrap());

    assert_eq!(view.cpu, "100.00%");
}

#[test]
fn host_temperatures_are_formatted_separately() {
    let view = PerformancePresentation::new(
        Some(GameTelemetry {
            cpu_temperature_millicelsius: Some(62_345),
            gpu_temperature_millicelsius: Some(70_000),
            ..GameTelemetry::default()
        }),
        NonZeroUsize::MIN,
    );

    assert_eq!(view.host_cpu_temperature, "62.3 °C");
    assert_eq!(view.host_gpu_temperature, "70.0 °C");
}

#[test]
fn clock_formats_local_time_and_repaints_at_the_next_minute() {
    let now = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 8, 42)
        .single()
        .expect("the fixed local timestamp should be unambiguous")
        .with_nanosecond(250_000_000)
        .expect("the fixed nanoseconds should be valid");

    let view = ClockPresentation::new(now, true);

    assert_eq!(view.time, "14:08");
    assert_eq!(view.date.as_deref(), Some("17/07/2026"));
    assert_eq!(view.repaint_after, Duration::from_millis(17_750));
}

#[test]
fn clock_can_hide_the_date_without_affecting_time() {
    let now = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 8, 42)
        .single()
        .expect("the fixed local timestamp should be unambiguous");

    let view = ClockPresentation::new(now, false);

    assert_eq!(view.time, "14:08");
    assert_eq!(view.date, None);
}

#[test]
fn clock_at_an_exact_minute_repaints_after_a_full_minute() {
    let now = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 9, 0)
        .single()
        .expect("the fixed local timestamp should be unambiguous");

    assert_eq!(
        ClockPresentation::new(now, true).repaint_after,
        Duration::from_secs(60)
    );
}

#[test]
fn clock_one_nanosecond_before_a_minute_repaints_after_one_nanosecond() {
    let now = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 8, 59)
        .single()
        .expect("the fixed local timestamp should be unambiguous")
        .with_nanosecond(999_999_999)
        .expect("the fixed nanoseconds should be valid");

    assert_eq!(
        ClockPresentation::new(now, true).repaint_after,
        Duration::from_nanos(1)
    );
}

#[test]
fn performance_layout_is_one_full_row_or_one_full_column() {
    let render = |layout| {
        let context = egui::Context::default();
        install_fonts(&context);
        install_theme(&context);
        let mut size = egui::Vec2::ZERO;
        let _ = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0))),
                ..RawInput::default()
            },
            |ui| {
                size = super::performance::paint_performance(
                    ui,
                    pos2(24.0, 24.0),
                    Some(telemetry(12_345, 3 * 1024 * 1024 * 1024)),
                    layout,
                    1.0,
                    0.0,
                    false,
                    false,
                    false,
                    24.0,
                )
                .size;
            },
        );
        size
    };
    let horizontal = render(PerformanceLayout::Horizontal);
    let vertical = render(PerformanceLayout::Vertical);

    assert!(horizontal.x > vertical.x, "{horizontal:?} {vertical:?}");
    assert!(horizontal.y < vertical.y, "{horizontal:?} {vertical:?}");
}

#[test]
fn horizontal_performance_uses_two_columns_only_below_the_safe_width() {
    assert_eq!(super::performance::performance_columns(300.0), 2);
    assert_eq!(super::performance::performance_columns(559.0), 2);
    assert_eq!(super::performance::performance_columns(560.0), 4);
    assert_eq!(super::performance::performance_columns(900.0), 4);
}

#[test]
fn performance_layouts_fit_narrow_and_medium_viewports() {
    const MARGIN: f32 = 24.0;

    for viewport in [vec2(300.0, 480.0), vec2(640.0, 480.0)] {
        for layout in [PerformanceLayout::Horizontal, PerformanceLayout::Vertical] {
            let context = egui::Context::default();
            install_fonts(&context);
            install_theme(&context);
            let mut size = egui::Vec2::ZERO;
            let _ = context.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), viewport)),
                    ..RawInput::default()
                },
                |ui| {
                    size = super::performance::paint_performance(
                        ui,
                        pos2(MARGIN, MARGIN),
                        Some(telemetry(12_345, 3 * 1024 * 1024 * 1024)),
                        layout,
                        1.0,
                        0.0,
                        false,
                        false,
                        false,
                        MARGIN,
                    )
                    .size;
                },
            );

            let safe_width = viewport.x - MARGIN * 2.0;
            assert!(
                size.x <= safe_width + 0.5,
                "{viewport:?} {layout:?}: {size:?}"
            );
        }
    }
}

#[test]
fn resizable_performance_width_matches_the_requested_outer_width() {
    for transparent_background in [true, false] {
        let context = egui::Context::default();
        install_fonts(&context);
        install_theme(&context);
        let mut size = egui::Vec2::ZERO;
        let _ = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0))),
                ..RawInput::default()
            },
            |ui| {
                size = super::performance::paint_performance(
                    ui,
                    pos2(24.0, 24.0),
                    Some(telemetry(12_345, 3 * 1024 * 1024 * 1024)),
                    PerformanceLayout::Horizontal,
                    1.0,
                    600.0,
                    transparent_background,
                    false,
                    true,
                    24.0,
                )
                .size;
            },
        );

        assert_eq!(
            size.x, 600.0,
            "transparent={transparent_background}: {size:?}"
        );
    }
}

#[test]
fn compact_common_widgets_do_not_paint_redundant_headers() {
    let context = egui::Context::default();
    context.enable_accesskit();
    install_fonts(&context);
    install_theme(&context);
    let output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1_920.0, 1_080.0))),
            ..RawInput::default()
        },
        |ui| {
            super::session::paint_session(
                ui,
                pos2(24.0, 24.0),
                Some(Duration::from_secs(90)),
                1.0,
                false,
                false,
                24.0,
            );
            super::clock::paint_clock(ui, pos2(360.0, 24.0), true, 1.0, false, false, 24.0);
            super::performance::paint_performance(
                ui,
                pos2(24.0, 220.0),
                None,
                PerformanceLayout::Horizontal,
                1.0,
                0.0,
                false,
                false,
                false,
                24.0,
            );
            super::manual_stopwatch::paint_manual_stopwatch(
                ui,
                pos2(720.0, 24.0),
                Duration::from_secs(90),
                true,
                OverlayMode::Interactive,
                1.0,
                false,
                false,
                true,
                24.0,
            );
        },
    );
    let text = painted_text(&output);
    assert!(
        text.iter().any(|value| value == "00:01:30.00"),
        "manual stopwatch did not render: {text:?}"
    );

    for redundant in [
        "SESSION",
        "GAME TIME",
        "Time in the current game process",
        "LOCAL TIME",
        "YOUR TIMEZONE",
        "PERFORMANCE",
        "LIVE SYSTEM SNAPSHOT",
        "STOPWATCH",
        "MANUAL TIMER",
        "RUNNING",
        "PAUSED",
    ] {
        assert!(!text.iter().any(|value| value == redundant), "{text:?}");
    }
}

fn painted_text(output: &egui::FullOutput) -> Vec<String> {
    let mut text = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(update) = &output.platform_output.accesskit_update {
        for (_, node) in &update.nodes {
            if let Some(label) = node.label() {
                text.push(label.to_owned());
            }
            if let Some(value) = node.value() {
                text.push(value.to_owned());
            }
        }
    }
    text
}
