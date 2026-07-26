use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{
    Event, RawInput, Rect,
    accesskit::{Action as AccessKitAction, ActionRequest, TreeId},
    pos2, vec2,
};
use overcrow_config::{TWITCH_PASSIVE_LIFETIME_DEFAULT_SECS, TwitchPrefs, WIDGET_PANEL_MIN};
use overcrow_protocol::OverlayMode;

use super::twitch_chat::{
    TwitchChatAction, TwitchWidgetState, paint_favorite_indicator, paint_header, paint_twitch_chat,
    paint_twitch_options, passive_message_alpha, twitch_passive_repaint_after, username_text,
};
use crate::twitch::model::{
    TwitchConnectionState, TwitchMessage, TwitchSendReceipt, TwitchSendReceiptState,
    TwitchSendState, TwitchSnapshot,
};

fn painted_text(output: &eframe::egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            eframe::egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

fn message(id: &str, age: Duration, now: Instant) -> TwitchMessage {
    TwitchMessage {
        id: id.to_owned(),
        display_name: "Alice".to_owned(),
        name_color: Some([145, 255, 0]),
        text: format!("message {id}"),
        reply: None,
        received_at: now - age,
        client_nonce: None,
        send_state: TwitchSendState::Received,
    }
}

fn paint_twitch_widget(
    state: &mut TwitchWidgetState,
    prefs: &TwitchPrefs,
) -> eframe::egui::FullOutput {
    paint_twitch_widget_at_size(state, prefs, vec2(420.0, 360.0))
}

fn paint_twitch_widget_at_size(
    state: &mut TwitchWidgetState,
    prefs: &TwitchPrefs,
    panel_size: eframe::egui::Vec2,
) -> eframe::egui::FullOutput {
    let context = eframe::egui::Context::default();
    context.enable_accesskit();
    context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..RawInput::default()
        },
        |ui| {
            let _ = paint_twitch_chat(
                ui,
                pos2(24.0, 24.0),
                panel_size,
                state,
                prefs,
                1.0,
                OverlayMode::Interactive,
                false,
                false,
                true,
                24.0,
                Instant::now(),
            );
        },
    )
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

fn accessible_bounds(output: &eframe::egui::FullOutput, label: &str) -> Rect {
    let bounds = output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accessibility tree")
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
        .unwrap_or_else(|| panic!("missing accessible bounds for {label:?}"));
    Rect::from_min_max(
        pos2(bounds.x0 as f32, bounds.y0 as f32),
        pos2(bounds.x1 as f32, bounds.y1 as f32),
    )
}

fn painted_bounds(output: &eframe::egui::FullOutput, label: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            eframe::egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing painted text {label:?}: {:?}", painted_text(output)))
}

#[test]
fn twitch_username_is_exactly_bold_text_without_an_icon_prefix() {
    let now = Instant::now();
    let message = message("1", Duration::ZERO, now);

    assert_eq!(
        username_text(&message, 1.0),
        eframe::egui::RichText::new("Alice:")
            .strong()
            .color(eframe::egui::Color32::from_rgb(145, 255, 0))
    );
}

#[test]
fn twitch_passive_messages_expire_and_fade_near_their_deadline() {
    let lifetime = Duration::from_secs(TWITCH_PASSIVE_LIFETIME_DEFAULT_SECS.into());

    assert_eq!(
        passive_message_alpha(Duration::from_secs(1), lifetime),
        Some(1.0)
    );
    let faded = passive_message_alpha(Duration::from_secs(25), lifetime).unwrap();
    assert!(faded > 0.0 && faded < 1.0);
    assert_eq!(
        passive_message_alpha(Duration::from_secs(31), lifetime),
        None
    );
}

#[test]
fn twitch_passive_repaint_stops_after_recent_messages_expire() {
    let now = Instant::now();
    let lifetime = Duration::from_secs(TWITCH_PASSIVE_LIFETIME_DEFAULT_SECS.into());
    let recent = TwitchSnapshot {
        messages: vec![message("1", Duration::from_secs(1), now)],
        ..TwitchSnapshot::default()
    };
    let expired = TwitchSnapshot {
        messages: vec![message("1", Duration::from_secs(31), now)],
        ..TwitchSnapshot::default()
    };

    assert_eq!(
        twitch_passive_repaint_after(&recent, lifetime, now),
        Some(Duration::from_millis(100))
    );
    assert_eq!(twitch_passive_repaint_after(&expired, lifetime, now), None);
}

#[test]
fn twitch_widget_draft_is_bounded_to_500_characters() {
    let mut state = TwitchWidgetState::default();
    state.set_draft(format!("{}é", "x".repeat(500)));

    assert_eq!(state.draft().chars().count(), 500);
    assert!(!state.draft().ends_with('é'));
}

#[test]
fn twitch_channel_draft_stays_empty_after_the_user_clears_it() {
    let mut state = TwitchWidgetState::default();
    let prefs = TwitchPrefs {
        active_channel: Some("sardoche".to_owned()),
        ..TwitchPrefs::default()
    };

    state.sync_channel_draft(&prefs);
    assert_eq!(state.channel_draft(), "sardoche");

    // Simulate the user deleting the field contents, then another frame of paint.
    state.set_channel_draft("");
    state.sync_channel_draft(&prefs);
    assert!(
        state.channel_draft().is_empty(),
        "cleared channel input must not be refilled from the active channel"
    );
}

#[test]
fn twitch_options_show_bounded_settings_feedback() {
    let context = eframe::egui::Context::default();
    let mut state = TwitchWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            authenticated_login: Some("viewer".to_owned()),
            credentials_available: true,
            client_configured: true,
            ..TwitchSnapshot::default()
        }),
    );
    state.set_message(Some("Could not save Twitch widget settings.".to_owned()));
    let mut actions = Vec::new();

    let output = context.run_ui(eframe::egui::RawInput::default(), |ui| {
        paint_twitch_options(ui, &mut state, &TwitchPrefs::default(), &mut actions);
    });
    let text = painted_text(&output);

    assert!(
        text.iter()
            .any(|value| value == "Could not save Twitch widget settings."),
        "{text:?}"
    );
    assert!(!text.iter().any(|value| value == "Connect Twitch"));
    assert!(text.iter().any(|value| value == "Sign out of Twitch"));
    assert!(actions.is_empty());
}

#[test]
fn twitch_options_keep_only_account_logout_and_passive_lifetime() {
    let context = eframe::egui::Context::default();
    let mut state = TwitchWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            authenticated_login: Some("viewer".to_owned()),
            credentials_available: true,
            client_configured: true,
            ..TwitchSnapshot::default()
        }),
    );
    let mut actions = Vec::new();

    let output = context.run_ui(eframe::egui::RawInput::default(), |ui| {
        paint_twitch_options(ui, &mut state, &TwitchPrefs::default(), &mut actions);
    });
    let text = painted_text(&output);

    assert!(text.iter().any(|value| value == "Sign out of Twitch"));
    assert!(text.iter().any(|value| value == "Passive lifetime"));
    assert!(!text.iter().any(|value| value == "Connect Twitch"));
    assert!(!text.iter().any(|value| value == "Join chat"));
    assert!(!text.iter().any(|value| value == "CHANNEL"));
}

#[test]
fn disconnected_twitch_widget_offers_authentication_in_its_content() {
    let mut state = TwitchWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            client_configured: true,
            ..TwitchSnapshot::default()
        }),
    );

    let output = paint_twitch_widget(&mut state, &TwitchPrefs::default());
    let text = accessible_text(&output);

    assert!(
        text.iter().any(|value| value == "Connect Twitch"),
        "{text:?}"
    );
    assert!(!text.iter().any(|value| value == "Join chat"), "{text:?}");
}

#[test]
fn authenticated_twitch_widget_offers_channel_join_and_favorites() {
    let mut state = TwitchWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            authenticated_login: Some("viewer".to_owned()),
            credentials_available: true,
            client_configured: true,
            ..TwitchSnapshot::default()
        }),
    );
    let prefs = TwitchPrefs {
        favorites: vec!["playervox".to_owned(), "warframe".to_owned()],
        ..TwitchPrefs::default()
    };

    let output = paint_twitch_widget(&mut state, &prefs);
    let text = accessible_text(&output);

    assert!(text.iter().any(|value| value == "Join chat"), "{text:?}");
    assert!(text.iter().any(|value| value == "#playervox"), "{text:?}");
    assert!(text.iter().any(|value| value == "#warframe"), "{text:?}");
}

#[test]
fn joined_twitch_widget_replaces_channel_selection_with_chat_and_composer() {
    let now = Instant::now();
    let mut state = TwitchWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            channel: Some("playervox".to_owned()),
            connection: TwitchConnectionState::Joined,
            authenticated_login: Some("viewer".to_owned()),
            credentials_available: true,
            client_configured: true,
            messages: vec![message("visible", Duration::ZERO, now)],
            ..TwitchSnapshot::default()
        }),
    );
    let prefs = TwitchPrefs {
        active_channel: Some("playervox".to_owned()),
        ..TwitchPrefs::default()
    };

    let output = paint_twitch_widget(&mut state, &prefs);
    let text = accessible_text(&output);

    assert!(
        text.iter().any(|value| value == "Disconnect channel"),
        "{text:?}"
    );
    assert!(text.iter().any(|value| value == "Send"), "{text:?}");
    assert!(
        text.iter().any(|value| value == "message visible"),
        "{text:?}"
    );
    assert!(!text.iter().any(|value| value == "Join chat"), "{text:?}");
    assert!(!text.iter().any(|value| value == "CHANNEL"), "{text:?}");
}

#[test]
fn joined_twitch_composer_keeps_send_and_disconnect_inside_a_narrow_widget() {
    let mut state = TwitchWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            channel: Some("playervox".to_owned()),
            connection: TwitchConnectionState::Joined,
            authenticated_login: Some("viewer".to_owned()),
            credentials_available: true,
            client_configured: true,
            ..TwitchSnapshot::default()
        }),
    );
    let prefs = TwitchPrefs {
        active_channel: Some("playervox".to_owned()),
        ..TwitchPrefs::default()
    };

    let panel = Rect::from_min_size(pos2(24.0, 24.0), vec2(WIDGET_PANEL_MIN, 360.0));
    let output = paint_twitch_widget_at_size(&mut state, &prefs, panel.size());
    for label in ["Send", "Favorite #playervox", "Disconnect channel"] {
        let bounds = accessible_bounds(&output, label);
        assert!(
            panel.expand(0.5).contains_rect(bounds),
            "{label} escaped the widget: {bounds:?}"
        );
    }
}

#[test]
fn disconnect_channel_control_emits_only_the_clear_channel_action() {
    assert_eq!(
        activate_disconnect_channel(true),
        [TwitchChatAction::ClearChannel]
    );
}

#[test]
fn modal_blocked_twitch_widget_ignores_disconnect_activation() {
    assert!(activate_disconnect_channel(false).is_empty());
}

fn activate_disconnect_channel(input_enabled: bool) -> Vec<TwitchChatAction> {
    let context = eframe::egui::Context::default();
    context.enable_accesskit();
    let mut state = TwitchWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            channel: Some("playervox".to_owned()),
            connection: TwitchConnectionState::Joined,
            authenticated_login: Some("viewer".to_owned()),
            credentials_available: true,
            client_configured: true,
            ..TwitchSnapshot::default()
        }),
    );
    let prefs = TwitchPrefs {
        active_channel: Some("playervox".to_owned()),
        ..TwitchPrefs::default()
    };
    let first = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..RawInput::default()
        },
        |ui| {
            let _ = paint_twitch_chat(
                ui,
                pos2(24.0, 24.0),
                vec2(420.0, 360.0),
                &mut state,
                &prefs,
                1.0,
                OverlayMode::Interactive,
                false,
                false,
                input_enabled,
                24.0,
                Instant::now(),
            );
        },
    );
    let disconnect = first
        .platform_output
        .accesskit_update
        .expect("accessibility tree")
        .nodes
        .into_iter()
        .find_map(|(id, node)| (node.label() == Some("Disconnect channel")).then_some(id))
        .expect("disconnect channel control");
    let mut actions = Vec::new();
    let _ = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            events: vec![Event::AccessKitActionRequest(ActionRequest {
                action: AccessKitAction::Click,
                target_tree: TreeId::ROOT,
                target_node: disconnect,
                data: None,
            })],
            ..RawInput::default()
        },
        |ui| {
            actions = paint_twitch_chat(
                ui,
                pos2(24.0, 24.0),
                vec2(420.0, 360.0),
                &mut state,
                &prefs,
                1.0,
                OverlayMode::Interactive,
                false,
                false,
                input_enabled,
                24.0,
                Instant::now(),
            )
            .actions;
        },
    );

    actions
}

#[test]
fn twitch_favorite_header_indicator_is_static() {
    let context = eframe::egui::Context::default();
    let prefs = TwitchPrefs {
        active_channel: Some("playervox".to_owned()),
        favorites: vec!["playervox".to_owned()],
        ..TwitchPrefs::default()
    };

    let output = context.run_ui(eframe::egui::RawInput::default(), |ui| {
        paint_favorite_indicator(ui, &prefs);
    });
    let text = painted_text(&output);

    assert_eq!(text.iter().filter(|value| value.as_str() == "★").count(), 1);
}

#[test]
fn twitch_favorite_star_precedes_the_channel_on_the_same_line() {
    let context = eframe::egui::Context::default();
    context.enable_accesskit();
    let mut state = TwitchWidgetState::default();
    state.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            channel: Some("playervox".to_owned()),
            connection: TwitchConnectionState::Joined,
            authenticated_login: Some("viewer".to_owned()),
            credentials_available: true,
            client_configured: true,
            ..TwitchSnapshot::default()
        }),
    );
    let prefs = TwitchPrefs {
        active_channel: Some("playervox".to_owned()),
        favorites: vec!["playervox".to_owned()],
        ..TwitchPrefs::default()
    };
    let output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..RawInput::default()
        },
        |ui| {
            paint_header(ui, &mut state, &prefs);
        },
    );
    let star = painted_bounds(&output, "★");
    let channel = painted_bounds(&output, "#playervox");

    assert!(star.right() <= channel.left(), "{star:?} {channel:?}");
    let star_center = star.center().y;
    let channel_center = channel.center().y;
    assert!((star_center - channel_center).abs() <= 2.0);
}

#[test]
fn twitch_widget_counts_unread_only_while_scrolled_away_from_bottom() {
    let now = Instant::now();
    let mut state = TwitchWidgetState::default();
    let mut snapshot = TwitchSnapshot {
        generation: 1,
        messages: vec![message("1", Duration::ZERO, now)],
        ..TwitchSnapshot::default()
    };
    state.apply_snapshot(1, Arc::new(snapshot.clone()));
    assert_eq!(state.unread_count(), 0);

    state.set_auto_scroll(false);
    snapshot.messages.push(message("2", Duration::ZERO, now));
    state.apply_snapshot(2, Arc::new(snapshot));
    assert_eq!(state.unread_count(), 1);

    state.return_to_latest();
    assert_eq!(state.unread_count(), 0);
    assert!(state.auto_scroll());
}

#[test]
fn twitch_widget_counts_new_messages_when_the_ring_is_already_full() {
    let now = Instant::now();
    let mut state = TwitchWidgetState::default();
    let messages = (0..200)
        .map(|index| message(&index.to_string(), Duration::ZERO, now))
        .collect::<Vec<_>>();
    state.apply_snapshot(
        1,
        Arc::new(TwitchSnapshot {
            generation: 1,
            messages: messages.clone(),
            ..TwitchSnapshot::default()
        }),
    );
    state.set_auto_scroll(false);

    let mut shifted = messages[1..].to_vec();
    shifted.push(message("200", Duration::ZERO, now));
    state.apply_snapshot(
        2,
        Arc::new(TwitchSnapshot {
            generation: 1,
            messages: shifted,
            ..TwitchSnapshot::default()
        }),
    );

    assert_eq!(state.unread_count(), 1);
}

#[test]
fn twitch_channel_generation_clears_reply_draft_and_unread() {
    let now = Instant::now();
    let mut state = TwitchWidgetState::default();
    state.set_draft("hello".to_owned());
    state.set_reply("parent".to_owned(), "Alice".to_owned());
    state.set_auto_scroll(false);
    let snapshot = TwitchSnapshot {
        generation: 2,
        messages: vec![message("1", Duration::ZERO, now)],
        ..TwitchSnapshot::default()
    };
    state.apply_snapshot(1, Arc::new(snapshot));

    assert!(state.draft().is_empty());
    assert!(state.reply_target().is_none());
    assert_eq!(state.unread_count(), 0);
}

#[test]
fn twitch_draft_clears_only_after_worker_acceptance() {
    let mut state = TwitchWidgetState::default();
    let snapshot = TwitchSnapshot {
        generation: 3,
        channel: Some("warframe".to_owned()),
        ..TwitchSnapshot::default()
    };
    state.apply_snapshot(1, Arc::new(snapshot.clone()));
    state.set_draft("hello".to_owned());
    let action = state.begin_send(&snapshot).expect("send can start");
    let TwitchChatAction::Command(crate::twitch::model::TwitchCommand::SendMessage {
        request_id,
        ..
    }) = action
    else {
        panic!("expected send action");
    };
    assert_eq!(state.draft(), "hello");

    state.apply_snapshot(
        2,
        Arc::new(TwitchSnapshot {
            send_receipt: Some(TwitchSendReceipt {
                request_id,
                state: TwitchSendReceiptState::Accepted,
            }),
            ..snapshot
        }),
    );
    assert!(state.draft().is_empty());
}

#[test]
fn twitch_rejected_send_keeps_the_draft_for_retry() {
    let mut state = TwitchWidgetState::default();
    let snapshot = TwitchSnapshot {
        generation: 3,
        channel: Some("warframe".to_owned()),
        ..TwitchSnapshot::default()
    };
    state.apply_snapshot(1, Arc::new(snapshot.clone()));
    state.set_draft("hello".to_owned());
    let TwitchChatAction::Command(crate::twitch::model::TwitchCommand::SendMessage {
        request_id,
        ..
    }) = state.begin_send(&snapshot).expect("send can start")
    else {
        panic!("expected send action");
    };

    state.apply_snapshot(
        2,
        Arc::new(TwitchSnapshot {
            send_receipt: Some(TwitchSendReceipt {
                request_id,
                state: TwitchSendReceiptState::Rejected,
            }),
            ..snapshot
        }),
    );
    assert_eq!(state.draft(), "hello");
}
