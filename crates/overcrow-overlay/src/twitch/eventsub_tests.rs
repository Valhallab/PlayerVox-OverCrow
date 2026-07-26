use std::time::Duration;

use super::{
    eventsub::{
        EVENTSUB_MESSAGE_MAX_BYTES, EventSubKind, EventSubParseError, EventSubRevocation,
        parse_eventsub_message,
    },
    model::{TwitchMessageFragment, TwitchReplyContext},
};

#[test]
fn eventsub_parses_welcome_keepalive_and_reconnect() {
    let welcome = parse_eventsub_message(
        r#"{
          "metadata":{"message_id":"welcome-1","message_type":"session_welcome"},
          "payload":{"session":{"id":"session-1","keepalive_timeout_seconds":10}}
        }"#,
    )
    .expect("welcome");
    assert_eq!(welcome.delivery_id, "welcome-1");
    assert_eq!(
        welcome.kind,
        EventSubKind::Welcome {
            session_id: "session-1".to_owned(),
            keepalive_timeout: Duration::from_secs(10),
        }
    );

    let keepalive = parse_eventsub_message(
        r#"{
          "metadata":{"message_id":"keepalive-1","message_type":"session_keepalive"},
          "payload":{}
        }"#,
    )
    .expect("keepalive");
    assert_eq!(keepalive.kind, EventSubKind::Keepalive);

    let reconnect = parse_eventsub_message(
        r#"{
          "metadata":{"message_id":"reconnect-1","message_type":"session_reconnect"},
          "payload":{"session":{
            "id":"session-2",
            "reconnect_url":"wss://eventsub.wss.twitch.tv/ws?reconnect=opaque"
          }}
        }"#,
    )
    .expect("reconnect");
    assert_eq!(
        reconnect.kind,
        EventSubKind::Reconnect {
            url: "wss://eventsub.wss.twitch.tv/ws?reconnect=opaque".to_owned(),
        }
    );
}

#[test]
fn eventsub_parses_text_chat_and_reply() {
    let parsed = parse_eventsub_message(
        r##"{
          "metadata":{
            "message_id":"delivery-1",
            "message_type":"notification",
            "subscription_type":"channel.chat.message",
            "subscription_version":"1"
          },
          "payload":{"event":{
            "broadcaster_user_id":"100",
            "chatter_user_id":"42",
            "chatter_user_login":"player_vox",
            "chatter_user_name":"PlayerVox",
            "message_id":"message-1",
            "color":"#91FF00",
            "message":{
              "text":"Hello chat!",
              "fragments":[{"type":"text","text":"Hello chat!","cheermote":null,"emote":null}]
            },
            "badges":[{"set_id":"subscriber","id":"12","info":"12"}],
            "reply":{
              "parent_message_id":"parent-1",
              "parent_message_body":"Earlier message",
              "parent_user_name":"Alice"
            }
          }}
        }"##,
    )
    .expect("chat message");

    let EventSubKind::ChatMessage {
        broadcaster_user_id,
        message,
    } = parsed.kind
    else {
        panic!("chat event");
    };
    assert_eq!(broadcaster_user_id, "100");
    assert_eq!(message.id, "message-1");
    assert_eq!(message.display_name, "PlayerVox");
    assert_eq!(message.name_color, Some([0x91, 0xff, 0x00]));
    assert_eq!(message.text, "Hello chat!");
    assert_eq!(
        message.fragments,
        vec![TwitchMessageFragment::Text("Hello chat!".to_owned())]
    );
    assert_eq!(
        message.reply,
        Some(TwitchReplyContext {
            message_id: "parent-1".to_owned(),
            display_name: "Alice".to_owned(),
            body: "Earlier message".to_owned(),
        })
    );
}

#[test]
fn eventsub_preserves_native_twitch_emote_fragments() {
    let parsed = parse_eventsub_message(
        r#"{
          "metadata":{
            "message_id":"delivery-emote",
            "message_type":"notification",
            "subscription_type":"channel.chat.message",
            "subscription_version":"1"
          },
          "payload":{"event":{
            "broadcaster_user_id":"100",
            "chatter_user_login":"alice",
            "chatter_user_name":"Alice",
            "message_id":"message-emote",
            "message":{
              "text":"Hello LUL!",
              "fragments":[
                {"type":"text","text":"Hello ","emote":null},
                {"type":"emote","text":"LUL","emote":{"id":"425618","emote_set_id":"0","owner_id":"0","format":["static"]}},
                {"type":"text","text":"!","emote":null}
              ]
            }
          }}
        }"#,
    )
    .expect("chat message with emote");

    let EventSubKind::ChatMessage { message, .. } = parsed.kind else {
        panic!("chat event");
    };
    assert_eq!(
        message.fragments,
        vec![
            TwitchMessageFragment::Text("Hello ".to_owned()),
            TwitchMessageFragment::Emote {
                id: "425618".to_owned(),
                alt: "LUL".to_owned(),
            },
            TwitchMessageFragment::Text("!".to_owned()),
        ]
    );
}

#[test]
fn malformed_optional_fragments_fall_back_to_bounded_message_text() {
    for fragments in [
        serde_json::json!({"not": "a list"}),
        serde_json::json!([{"type": "emote", "text": "LUL", "emote": {"id": "../bad"}}]),
        serde_json::json!(
            (0..65)
                .map(|_| serde_json::json!({"type": "text", "text": "x"}))
                .collect::<Vec<_>>()
        ),
    ] {
        let raw = serde_json::json!({
            "metadata": {
                "message_id": "delivery-fragment-fallback",
                "message_type": "notification",
                "subscription_type": "channel.chat.message",
                "subscription_version": "1"
            },
            "payload": {
                "event": {
                    "broadcaster_user_id": "100",
                    "chatter_user_login": "alice",
                    "chatter_user_name": "Alice",
                    "message_id": "message-fragment-fallback",
                    "message": {
                        "text": "Safe fallback",
                        "fragments": fragments
                    }
                }
            }
        })
        .to_string();
        let parsed = parse_eventsub_message(&raw).expect("valid main message");
        let EventSubKind::ChatMessage { message, .. } = parsed.kind else {
            panic!("chat event");
        };
        assert_eq!(
            message.fragments,
            vec![TwitchMessageFragment::Text("Safe fallback".to_owned())]
        );
    }
}

#[test]
fn eventsub_parses_chat_without_optional_name_color() {
    let parsed = parse_eventsub_message(
        r#"{
          "metadata":{
            "message_id":"delivery-2",
            "message_type":"notification",
            "subscription_type":"channel.chat.message",
            "subscription_version":"1"
          },
          "payload":{"event":{
            "broadcaster_user_id":"100",
            "chatter_user_id":"42",
            "chatter_user_login":"player_vox",
            "chatter_user_name":"PlayerVox",
            "message_id":"message-2",
            "color":null,
            "message":{"text":"Hello without a color"},
            "reply":null
          }}
        }"#,
    )
    .expect("chat message");

    let EventSubKind::ChatMessage { message, .. } = parsed.kind else {
        panic!("chat event");
    };
    assert_eq!(message.name_color, None);
    assert_eq!(message.text, "Hello without a color");
    assert_eq!(
        message.fragments,
        vec![TwitchMessageFragment::Text(
            "Hello without a color".to_owned()
        )]
    );
}

#[test]
fn eventsub_accepts_bounded_opaque_delivery_ids() {
    let raw = r#"{
          "metadata":{
            "message_id":"opaque:delivery+/=",
            "message_type":"notification",
            "subscription_type":"channel.chat.message",
            "subscription_version":"1"
          },
          "payload":{"event":{
            "broadcaster_user_id":"100",
            "chatter_user_login":"alice",
            "chatter_user_name":"Alice",
            "message_id":"message-opaque-delivery",
            "message":{"text":"Hello from Twitch"}
          }}
        }"#;
    let parsed = parse_eventsub_message(raw).expect("bounded opaque delivery ID");

    assert_eq!(parsed.delivery_id, "opaque:delivery+/=");
    assert!(matches!(parsed.kind, EventSubKind::ChatMessage { .. }));

    let control_id = raw.replace("opaque:delivery+/=", r"bad\ndelivery");
    assert_eq!(
        parse_eventsub_message(&control_id),
        Err(EventSubParseError::InvalidEnvelope)
    );

    let oversized_id = raw.replace("opaque:delivery+/=", &"x".repeat(257));
    assert_eq!(
        parse_eventsub_message(&oversized_id),
        Err(EventSubParseError::InvalidEnvelope)
    );
}

#[test]
fn eventsub_preserves_valid_chat_when_display_metadata_needs_normalization() {
    let parsed = parse_eventsub_message(
        r#"{
          "metadata":{
            "message_id":"delivery-normalized",
            "message_type":"notification",
            "subscription_type":"channel.chat.message",
            "subscription_version":"1"
          },
          "payload":{"event":{
            "broadcaster_user_id":"100",
            "chatter_user_id":"42",
            "chatter_user_login":"alice_login",
            "chatter_user_name":"",
            "message_id":"message-normalized",
            "color":"",
            "message":{"text":"Hello\u0001chat"},
            "reply":{
              "parent_message_id":"",
              "parent_message_body":"Invalid optional reply",
              "parent_user_name":"Alice"
            }
          }}
        }"#,
    )
    .expect("valid main chat message");

    let EventSubKind::ChatMessage { message, .. } = parsed.kind else {
        panic!("chat event");
    };
    assert_eq!(message.display_name, "alice_login");
    assert_eq!(message.text, "Hello chat");
    assert_eq!(message.reply, None);
}

#[test]
fn eventsub_discards_structurally_invalid_optional_replies() {
    for (index, reply) in [
        serde_json::json!([]),
        serde_json::json!({"parent_message_id": 42}),
    ]
    .into_iter()
    .enumerate()
    {
        let raw = serde_json::json!({
            "metadata": {
                "message_id": format!("delivery-invalid-reply-{index}"),
                "message_type": "notification",
                "subscription_type": "channel.chat.message",
                "subscription_version": "1"
            },
            "payload": {
                "event": {
                    "broadcaster_user_id": "100",
                    "chatter_user_login": "alice",
                    "chatter_user_name": "Alice",
                    "message_id": format!("message-invalid-reply-{index}"),
                    "message": {"text": "Valid main message"},
                    "reply": reply
                }
            }
        })
        .to_string();

        let parsed = parse_eventsub_message(&raw).expect("valid main chat message");
        let EventSubKind::ChatMessage { message, .. } = parsed.kind else {
            panic!("chat event");
        };
        assert_eq!(message.text, "Valid main message");
        assert_eq!(message.reply, None);
    }
}

#[test]
fn eventsub_bounds_display_text_without_relaxing_message_identity() {
    let raw = serde_json::json!({
        "metadata": {
            "message_id": "delivery-bounded-text",
            "message_type": "notification",
            "subscription_type": "channel.chat.message",
            "subscription_version": "1"
        },
        "payload": {
            "event": {
                "broadcaster_user_id": "100",
                "chatter_user_name": "N".repeat(129),
                "message_id": "message-bounded-text",
                "message": {"text": "M".repeat(501)}
            }
        }
    })
    .to_string();
    let parsed = parse_eventsub_message(&raw).expect("bounded display text");
    let EventSubKind::ChatMessage { message, .. } = parsed.kind else {
        panic!("chat event");
    };
    assert_eq!(message.display_name.chars().count(), 128);
    assert_eq!(message.text.chars().count(), 500);

    let invalid_id = raw.replace("message-bounded-text", "message/not-allowed");
    assert_eq!(
        parse_eventsub_message(&invalid_id),
        Err(EventSubParseError::InvalidIdentity)
    );
}

#[test]
fn eventsub_parses_clear_delete_and_revocation() {
    let clear = parse_eventsub_message(
        r#"{
          "metadata":{
            "message_id":"clear-1",
            "message_type":"notification",
            "subscription_type":"channel.chat.clear",
            "subscription_version":"1"
          },
          "payload":{"event":{"broadcaster_user_id":"100"}}
        }"#,
    )
    .expect("clear");
    assert_eq!(
        clear.kind,
        EventSubKind::ChatClear {
            broadcaster_user_id: "100".to_owned(),
        }
    );

    let delete = parse_eventsub_message(
        r#"{
          "metadata":{
            "message_id":"delete-1",
            "message_type":"notification",
            "subscription_type":"channel.chat.message_delete",
            "subscription_version":"1"
          },
          "payload":{"event":{
            "broadcaster_user_id":"100",
            "message_id":"message-1"
          }}
        }"#,
    )
    .expect("delete");
    assert_eq!(
        delete.kind,
        EventSubKind::ChatMessageDelete {
            broadcaster_user_id: "100".to_owned(),
            message_id: "message-1".to_owned(),
        }
    );

    let revoked = parse_eventsub_message(
        r#"{
          "metadata":{"message_id":"revoke-1","message_type":"revocation"},
          "payload":{"subscription":{"status":"authorization_revoked"}}
        }"#,
    )
    .expect("revocation");
    assert_eq!(
        revoked.kind,
        EventSubKind::Revocation(EventSubRevocation::Authentication)
    );
}

#[test]
fn eventsub_rejects_oversized_malformed_and_untrusted_reconnect_data() {
    let oversized = "x".repeat(EVENTSUB_MESSAGE_MAX_BYTES + 1);
    assert_eq!(
        parse_eventsub_message(&oversized),
        Err(EventSubParseError::Oversized)
    );

    for (invalid, expected) in [
        ("{}", EventSubParseError::Malformed),
        (
            r#"{"metadata":{"message_id":"","message_type":"session_keepalive"},"payload":{}}"#,
            EventSubParseError::InvalidEnvelope,
        ),
        (
            r#"{"metadata":{"message_id":"x","message_type":"session_welcome"},"payload":{"session":{"id":"s","keepalive_timeout_seconds":0}}}"#,
            EventSubParseError::InvalidEnvelope,
        ),
        (
            r#"{"metadata":{"message_id":"x","message_type":"session_reconnect"},"payload":{"session":{"id":"s","reconnect_url":"wss://evil.example/ws"}}}"#,
            EventSubParseError::InvalidRouting,
        ),
        (
            r#"{"metadata":{"message_id":"x","message_type":"notification","subscription_type":"channel.chat.message","subscription_version":"2"},"payload":{"event":{"broadcaster_user_id":"1","chatter_user_id":"2","chatter_user_login":"alice","chatter_user_name":"Alice","message_id":"m","color":"","message":{"text":"hello"}}}}"#,
            EventSubParseError::InvalidRouting,
        ),
        (
            r#"{"metadata":{"message_id":"x","message_type":"notification","subscription_type":"channel.chat.message","subscription_version":"1"},"payload":{"event":{"broadcaster_user_id":"not-an-id","chatter_user_id":"2","chatter_user_login":"alice","chatter_user_name":"Alice","message_id":"m","color":"","message":{"text":"hello"}}}}"#,
            EventSubParseError::InvalidIdentity,
        ),
    ] {
        assert_eq!(parse_eventsub_message(invalid), Err(expected), "{invalid}");
    }
}

#[test]
fn eventsub_classifies_invalid_values_without_exposing_them() {
    assert_eq!(
        parse_eventsub_message("{}"),
        Err(EventSubParseError::Malformed)
    );
    assert_eq!(
        parse_eventsub_message(
            r#"{"metadata":{"message_id":"","message_type":"session_keepalive"},"payload":{}}"#,
        ),
        Err(EventSubParseError::InvalidEnvelope)
    );
    assert_eq!(
        parse_eventsub_message(
            r#"{"metadata":{"message_id":"delivery","message_type":"notification","subscription_type":"channel.chat.message","subscription_version":"2"},"payload":{}}"#,
        ),
        Err(EventSubParseError::InvalidRouting)
    );
    assert_eq!(
        parse_eventsub_message(
            r#"{"metadata":{"message_id":"delivery","message_type":"notification","subscription_type":"channel.chat.message","subscription_version":"1"},"payload":{"event":{"broadcaster_user_id":"not-an-id","chatter_user_name":"Alice","message_id":"message","message":{"text":"hello"}}}}"#,
        ),
        Err(EventSubParseError::InvalidIdentity)
    );
    assert_eq!(
        parse_eventsub_message(
            r#"{"metadata":{"message_id":"delivery","message_type":"notification","subscription_type":"channel.chat.message","subscription_version":"1"},"payload":{"event":{"broadcaster_user_id":"100","chatter_user_name":"","message_id":"message","message":{"text":"hello"}}}}"#,
        ),
        Err(EventSubParseError::InvalidContent)
    );
}
