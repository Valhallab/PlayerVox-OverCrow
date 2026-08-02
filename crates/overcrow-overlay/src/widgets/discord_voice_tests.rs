use std::sync::Arc;

use eframe::egui::{Direction, RawInput, Rect, epaint::CircleShape, pos2, vec2};
use overcrow_config::DiscordVoiceAlignment;
use overcrow_protocol::OverlayMode;

use super::{
    WidgetGlyph,
    chrome::{ACCENT, PANEL_STROKE, apply_scale, install_theme},
    discord_voice::{
        DiscordVoicePresentation, DiscordWidgetState, VoiceStateIcon, discord_avatar_size,
        discord_voice_scroll_style, paint_discord_voice, paint_participant_content,
        participant_avatar_radius, participant_avatar_stroke, participant_row_layout,
        participant_row_size,
    },
};
use crate::{
    branding::install_fonts,
    discord::{
        client::{DiscordConnectionState, DiscordSnapshot},
        model::{VoiceChannel, VoiceParticipant},
    },
    icons::AppIcon,
};

#[test]
fn presentation_explains_every_connection_state_without_provider_details() {
    let cases = [
        (
            DiscordConnectionState::ClientNotConfigured,
            "Discord support is not configured in this build.",
        ),
        (
            DiscordConnectionState::Connecting,
            "Looking for the Discord desktop app…",
        ),
        (
            DiscordConnectionState::AuthorizationRequired,
            "Connect Discord to show your current voice channel.",
        ),
        (
            DiscordConnectionState::Authorizing,
            "Approve PlayerVox OverCrow in Discord.",
        ),
        (
            DiscordConnectionState::Authenticating,
            "Signing in to Discord…",
        ),
        (
            DiscordConnectionState::DiscordUnavailable,
            "Open the Discord desktop app to use voice overlay.",
        ),
        (
            DiscordConnectionState::Failed,
            "Discord credentials need attention. Try signing out again.",
        ),
    ];

    for (connection, expected) in cases {
        let presentation = DiscordVoicePresentation::new(
            &DiscordSnapshot {
                connection,
                ..DiscordSnapshot::default()
            },
            8,
        );
        assert_eq!(presentation.message, Some(expected));
    }
}

#[test]
fn failed_credential_state_has_an_actionable_message() {
    let presentation = DiscordVoicePresentation::new(
        &DiscordSnapshot {
            connection: DiscordConnectionState::Failed,
            ..DiscordSnapshot::default()
        },
        8,
    );

    assert_eq!(
        presentation.message,
        Some("Discord credentials need attention. Try signing out again.")
    );
}

#[test]
fn retained_channel_reports_resynchronization_and_stays_visible_in_passive() {
    let presentation = DiscordVoicePresentation::new(
        &DiscordSnapshot {
            connection: DiscordConnectionState::Connecting,
            channel: Some(VoiceChannel {
                id: "9".to_owned(),
                name: "Squad".to_owned(),
                participants: vec![VoiceParticipant::for_test("7", "Alice", false)],
            }),
            ..DiscordSnapshot::default()
        },
        8,
    );

    assert_eq!(presentation.message, Some("Resynchronizing…"));
    assert!(presentation.visible_in_passive());
}

#[test]
fn presentation_limits_participants_and_reports_the_remainder() {
    let snapshot = DiscordSnapshot {
        connection: DiscordConnectionState::Ready,
        channel: Some(VoiceChannel {
            id: "9".to_owned(),
            name: "Squad".to_owned(),
            participants: (0..5)
                .map(|index| {
                    VoiceParticipant::for_test(
                        &index.to_string(),
                        &format!("User {index}"),
                        index == 2,
                    )
                })
                .collect(),
        }),
        ..DiscordSnapshot::default()
    };

    let presentation = DiscordVoicePresentation::new(&snapshot, 3);

    assert!(presentation.has_channel);
    assert_eq!(presentation.participants.len(), 3);
    assert_eq!(presentation.overflow, 2);
    assert!(presentation.visible_in_passive());
}

#[test]
fn initials_are_safe_for_empty_and_multibyte_names() {
    assert_eq!(super::discord_voice::participant_initials(""), "?");
    assert_eq!(
        super::discord_voice::participant_initials("Alice Example"),
        "AE"
    );
    assert_eq!(super::discord_voice::participant_initials("Élodie"), "É");
}

#[test]
fn interactive_widget_offers_connect_but_passive_hides_disconnected_content() {
    let mut state = DiscordWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(DiscordSnapshot {
            connection: DiscordConnectionState::AuthorizationRequired,
            client_configured: true,
            ..DiscordSnapshot::default()
        }),
    );
    let interactive = paint_widget(&mut state, OverlayMode::Interactive);
    let labels = accessible_text(&interactive);

    assert!(
        labels.iter().any(|label| label == "Connect Discord"),
        "{labels:?}"
    );
    assert!(!state.visible_in_passive(8));
}

#[test]
fn connected_widget_contains_only_compact_participant_rows() {
    let mut state = DiscordWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(DiscordSnapshot {
            connection: DiscordConnectionState::Ready,
            channel: Some(VoiceChannel {
                id: "9".to_owned(),
                name: "Squad".to_owned(),
                participants: vec![
                    VoiceParticipant::for_test("7", "Alice", true),
                    VoiceParticipant::for_test("8", "Bob", false),
                ],
            }),
            ..DiscordSnapshot::default()
        }),
    );

    let output = paint_widget(&mut state, OverlayMode::Interactive);
    let labels = accessible_text(&output);

    for expected in ["Alice", "Bob"] {
        assert!(labels.iter().any(|label| label == expected), "{labels:?}");
    }
    for redundant in ["DISCORD VOICE", "Squad", "CONNECTED", "Speaking", "Idle"] {
        assert!(
            labels.iter().all(|label| label != redundant),
            "unexpected {redundant:?} in {labels:?}"
        );
    }
}

#[test]
fn connected_widget_exposes_dynamic_mute_and_deafen_icons() {
    let mut muted = VoiceParticipant::for_test("7", "Alice", false);
    muted.muted = true;
    let mut deafened = VoiceParticipant::for_test("8", "Bob", false);
    deafened.deafened = true;
    let participants = vec![muted.clone(), deafened.clone()];
    let mut state = DiscordWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(DiscordSnapshot {
            connection: DiscordConnectionState::Ready,
            channel: Some(VoiceChannel {
                id: "9".to_owned(),
                name: "Squad".to_owned(),
                participants: vec![muted, deafened],
            }),
            ..DiscordSnapshot::default()
        }),
    );

    let output = paint_widget(&mut state, OverlayMode::Interactive);
    let labels = accessible_text(&output);

    assert!(labels.iter().any(|label| label == "Muted"), "{labels:?}");
    assert!(labels.iter().any(|label| label == "Deafened"), "{labels:?}");
    assert_eq!(VoiceStateIcon::Muted.app_icon(), AppIcon::MicrophoneMuted);
    assert_eq!(VoiceStateIcon::Deafened.app_icon(), AppIcon::Headphones);

    let text = painted_text(&paint_participant_visuals(&mut state, &participants, 1.0));
    assert!(
        text.iter()
            .any(|value| value == AppIcon::MicrophoneMuted.glyph()),
        "{text:?}"
    );
    assert!(
        text.iter()
            .any(|value| value == AppIcon::Headphones.glyph()),
        "{text:?}"
    );
}

#[test]
fn passive_widget_paints_resynchronization_above_retained_participants() {
    let mut state = DiscordWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(DiscordSnapshot {
            connection: DiscordConnectionState::Connecting,
            channel: Some(VoiceChannel {
                id: "9".to_owned(),
                name: "Squad".to_owned(),
                participants: vec![VoiceParticipant::for_test("7", "Alice", false)],
            }),
            ..DiscordSnapshot::default()
        }),
    );

    let labels = accessible_text(&paint_widget(&mut state, OverlayMode::Passive));

    assert!(
        labels.iter().any(|label| label == "Resynchronizing…"),
        "{labels:?}"
    );
    assert!(labels.iter().any(|label| label == "Alice"), "{labels:?}");
}

#[test]
fn participant_metrics_follow_only_the_common_content_scale() {
    assert_eq!(discord_avatar_size(0.75), 26.25);
    assert_eq!(discord_avatar_size(1.0), 35.0);
    assert_eq!(discord_avatar_size(1.75), 61.25);

    let idle = participant_avatar_stroke(false, 1.0);
    let speaking = participant_avatar_stroke(true, 1.0);
    assert_eq!(idle.color, PANEL_STROKE);
    assert_eq!(speaking.color, ACCENT);
    assert!(speaking.width > idle.width);
    assert_eq!(participant_avatar_radius(35.0, 1.5), 16.75);
    assert!(participant_avatar_radius(35.0, speaking.width) + speaking.width * 0.5 <= 17.5);
}

#[test]
fn speaking_ring_stays_inside_the_painted_avatar_at_every_supported_scale() {
    for scale in [0.75, 1.0, 1.75] {
        let mut state = DiscordWidgetState::default();
        state.apply_snapshot(
            1,
            Arc::new(DiscordSnapshot {
                connection: DiscordConnectionState::Ready,
                channel: Some(VoiceChannel {
                    id: "9".to_owned(),
                    name: "Squad".to_owned(),
                    participants: vec![VoiceParticipant::for_test("7", "Alice", true)],
                }),
                ..DiscordSnapshot::default()
            }),
        );

        let participant = VoiceParticipant::for_test("7", "Alice", true);
        let output = paint_participant_visuals(&mut state, &[participant], scale);
        let circles = painted_circles(&output);
        let ring = circles
            .iter()
            .find(|circle| circle.stroke.color == ACCENT)
            .unwrap_or_else(|| panic!("missing speaking ring at scale {scale}: {circles:?}"));
        let avatar_bounds = Rect::from_center_size(
            ring.center,
            vec2(discord_avatar_size(scale), discord_avatar_size(scale)),
        );
        let ring_bounds = ring.visual_bounding_rect();

        assert_rect_approximately_equal(ring_bounds, avatar_bounds, scale);
    }
}

#[test]
fn participant_alignment_mirrors_the_row_direction() {
    assert_eq!(
        participant_row_layout(DiscordVoiceAlignment::Left).main_dir,
        Direction::LeftToRight
    );
    assert_eq!(
        participant_row_layout(DiscordVoiceAlignment::Right).main_dir,
        Direction::RightToLeft
    );
}

#[test]
fn right_aligned_rows_have_an_explicit_compact_extent() {
    assert_eq!(participant_row_size(320.0, 1.0), vec2(320.0, 35.0));
    assert_eq!(participant_row_size(320.0, 0.9), vec2(320.0, 31.5));
}

#[test]
fn discord_scrollbar_reserves_space_from_right_aligned_content() {
    let original = eframe::egui::style::ScrollStyle::floating();
    let reserved = discord_voice_scroll_style(original);

    assert!(!reserved.floating);
    assert!(reserved.allocated_width() >= original.bar_width);
}

#[test]
fn discord_catalog_glyph_uses_the_shared_discord_logo() {
    assert_eq!(WidgetGlyph::Discord.app_icon(), AppIcon::Discord);
    assert_eq!(
        WidgetGlyph::Discord.app_icon().glyph(),
        egui_phosphor::regular::DISCORD_LOGO
    );
}

#[test]
fn connected_widget_ignores_the_saved_editing_width() {
    for alignment in [DiscordVoiceAlignment::Left, DiscordVoiceAlignment::Right] {
        for mode in [OverlayMode::Interactive, OverlayMode::Passive] {
            let mut state = connected_state("Alice");
            let measured = paint_widget_size(&mut state, mode, alignment);

            assert!(
                measured.x < 300.0,
                "{alignment:?} {mode:?} width stayed wide: {measured:?}"
            );
        }
    }
}

#[test]
fn connected_widget_ignores_the_saved_editing_height() {
    for mode in [OverlayMode::Interactive, OverlayMode::Passive] {
        let mut state = connected_state("Alice");
        let measured = paint_widget_size_with_panel(
            &mut state,
            mode,
            DiscordVoiceAlignment::Left,
            vec2(360.0, 500.0),
        );

        assert!(
            measured.y < 500.0,
            "{mode:?} height stayed fixed: {measured:?}"
        );
    }
}

#[test]
fn connected_widget_keeps_the_same_compact_size_between_modes() {
    for alignment in [DiscordVoiceAlignment::Left, DiscordVoiceAlignment::Right] {
        let mut state = connected_state("Alice");
        let interactive = paint_widget_size(&mut state, OverlayMode::Interactive, alignment);
        let passive = paint_widget_size(&mut state, OverlayMode::Passive, alignment);

        assert_vec2_approximately_equal(interactive, passive);
    }
}

#[test]
fn discord_widget_does_not_expose_a_resize_grip() {
    let mut state = connected_state("Alice");
    let output = paint_widget(&mut state, OverlayMode::Interactive);
    let labels = accessible_text(&output);

    assert!(
        labels.iter().all(|label| label != "Resize widget"),
        "unexpected Discord resize grip: {labels:?}"
    );
}

#[test]
fn long_participant_names_keep_the_connected_widget_bounded() {
    let mut state = connected_state(
        "A participant name that is intentionally much too long for a compact voice overlay",
    );

    for alignment in [DiscordVoiceAlignment::Left, DiscordVoiceAlignment::Right] {
        let size = paint_widget_size(&mut state, OverlayMode::Interactive, alignment);
        assert!(
            size.x <= 320.0,
            "{alignment:?} widget is too wide: {size:?}"
        );
    }
}

fn connected_state(display_name: &str) -> DiscordWidgetState {
    let mut state = DiscordWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(DiscordSnapshot {
            connection: DiscordConnectionState::Ready,
            channel: Some(VoiceChannel {
                id: "9".to_owned(),
                name: "Squad".to_owned(),
                participants: vec![VoiceParticipant::for_test("7", display_name, true)],
            }),
            ..DiscordSnapshot::default()
        }),
    );
    state
}

fn paint_widget_size(
    state: &mut DiscordWidgetState,
    mode: OverlayMode,
    alignment: DiscordVoiceAlignment,
) -> eframe::egui::Vec2 {
    paint_widget_size_with_panel(state, mode, alignment, vec2(360.0, 240.0))
}

fn paint_widget_size_with_panel(
    state: &mut DiscordWidgetState,
    mode: OverlayMode,
    alignment: DiscordVoiceAlignment,
    panel_size: eframe::egui::Vec2,
) -> eframe::egui::Vec2 {
    let context = eframe::egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    let mut measured = vec2(0.0, 0.0);
    let _ = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..RawInput::default()
        },
        |ui| {
            measured = paint_discord_voice(
                ui,
                pos2(24.0, 24.0),
                panel_size,
                state,
                8,
                alignment,
                1.0,
                mode,
                false,
                false,
                true,
                24.0,
            )
            .size;
        },
    );
    measured
}

fn assert_vec2_approximately_equal(actual: eframe::egui::Vec2, expected: eframe::egui::Vec2) {
    const EPSILON: f32 = 0.001;
    assert!(
        (actual.x - expected.x).abs() <= EPSILON && (actual.y - expected.y).abs() <= EPSILON,
        "widget size changed between modes: {actual:?} != {expected:?}"
    );
}

fn paint_widget(state: &mut DiscordWidgetState, mode: OverlayMode) -> eframe::egui::FullOutput {
    paint_widget_aligned(state, mode, DiscordVoiceAlignment::Left)
}

fn paint_widget_aligned(
    state: &mut DiscordWidgetState,
    mode: OverlayMode,
    alignment: DiscordVoiceAlignment,
) -> eframe::egui::FullOutput {
    let context = eframe::egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    context.enable_accesskit();
    context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..RawInput::default()
        },
        |ui| {
            let _ = paint_discord_voice(
                ui,
                pos2(24.0, 24.0),
                vec2(360.0, 240.0),
                state,
                8,
                alignment,
                1.0,
                mode,
                false,
                false,
                true,
                24.0,
            );
        },
    )
}

fn paint_participant_visuals(
    state: &mut DiscordWidgetState,
    participants: &[VoiceParticipant],
    scale: f32,
) -> eframe::egui::FullOutput {
    let context = eframe::egui::Context::default();
    install_fonts(&context);
    install_theme(&context);
    context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..RawInput::default()
        },
        |ui| {
            apply_scale(ui, scale);
            for participant in participants {
                ui.horizontal(|ui| {
                    paint_participant_content(
                        ui,
                        state,
                        participant,
                        true,
                        scale,
                        std::time::Instant::now(),
                    );
                });
            }
        },
    )
}

fn painted_text(output: &eframe::egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            eframe::egui::Shape::Text(shape) => Some(shape.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

fn painted_circles(output: &eframe::egui::FullOutput) -> Vec<CircleShape> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            eframe::egui::Shape::Circle(circle) => Some(*circle),
            _ => None,
        })
        .collect()
}

fn assert_rect_approximately_equal(actual: Rect, expected: Rect, scale: f32) {
    const EPSILON: f32 = 0.001;
    for (actual, expected) in [
        (actual.min.x, expected.min.x),
        (actual.min.y, expected.min.y),
        (actual.max.x, expected.max.x),
        (actual.max.y, expected.max.y),
    ] {
        assert!(
            (actual - expected).abs() <= EPSILON,
            "ring escaped avatar at scale {scale}: {actual} != {expected}"
        );
    }
}

fn accessible_text(output: &eframe::egui::FullOutput) -> Vec<String> {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accessibility tree")
        .nodes
        .iter()
        .flat_map(|(_, node)| {
            [
                node.label().map(str::to_owned),
                node.value().map(str::to_owned),
            ]
        })
        .flatten()
        .collect()
}
