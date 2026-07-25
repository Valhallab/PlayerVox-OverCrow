use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use overcrow_config::{TWITCH_PASSIVE_LIFETIME_DEFAULT_SECS, TwitchPrefs};

use super::twitch_chat::{
    TwitchChatAction, TwitchWidgetState, passive_message_alpha, twitch_passive_repaint_after,
    username_text,
};
use crate::twitch::model::{
    TwitchMessage, TwitchSendReceipt, TwitchSendReceiptState, TwitchSendState, TwitchSnapshot,
};

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
