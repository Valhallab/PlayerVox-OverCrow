use std::time::Duration;

use super::{
    eventsub::{
        EVENTSUB_MESSAGE_MAX_BYTES, EventSubKind, EventSubParseError, EventSubRevocation,
        parse_eventsub_message,
    },
    model::TwitchReplyContext,
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
fn eventsub_parses_text_chat_and_reply_without_assets() {
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
        message.reply,
        Some(TwitchReplyContext {
            message_id: "parent-1".to_owned(),
            display_name: "Alice".to_owned(),
            body: "Earlier message".to_owned(),
        })
    );
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

    for invalid in [
        "{}",
        r#"{"metadata":{"message_id":"","message_type":"session_keepalive"},"payload":{}}"#,
        r#"{"metadata":{"message_id":"x","message_type":"session_welcome"},"payload":{"session":{"id":"s","keepalive_timeout_seconds":0}}}"#,
        r#"{"metadata":{"message_id":"x","message_type":"session_reconnect"},"payload":{"session":{"id":"s","reconnect_url":"wss://evil.example/ws"}}}"#,
        r#"{"metadata":{"message_id":"x","message_type":"notification","subscription_type":"channel.chat.message","subscription_version":"2"},"payload":{"event":{"broadcaster_user_id":"1","chatter_user_id":"2","chatter_user_login":"alice","chatter_user_name":"Alice","message_id":"m","color":"","message":{"text":"hello"}}}}"#,
        r#"{"metadata":{"message_id":"x","message_type":"notification","subscription_type":"channel.chat.message","subscription_version":"1"},"payload":{"event":{"broadcaster_user_id":"not-an-id","chatter_user_id":"2","chatter_user_login":"alice","chatter_user_name":"Alice","message_id":"m","color":"","message":{"text":"hello"}}}}"#,
    ] {
        assert_eq!(
            parse_eventsub_message(invalid),
            Err(EventSubParseError::Malformed),
            "{invalid}"
        );
    }
}
