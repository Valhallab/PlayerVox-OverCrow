use super::http::{
    ChatSendResult, ChatSubscription, HttpError, TwitchUser, parse_send_response,
    parse_subscription_response, parse_user_response,
};

#[test]
fn helix_user_response_requires_one_exact_normalized_login() {
    assert_eq!(
        parse_user_response(
            br#"{"data":[{"id":"100","login":"warframe","display_name":"Warframe"}]}"#,
            "warframe",
        ),
        Ok(TwitchUser {
            id: "100".to_owned(),
        })
    );

    for invalid in [
        br#"{"data":[]}"#.as_slice(),
        br#"{"data":[{"id":"100","login":"other"},{"id":"101","login":"warframe"}]}"#.as_slice(),
        br#"{"data":[{"id":"not-numeric","login":"warframe"}]}"#.as_slice(),
        br#"{"data":[{"id":"100","login":"other"}]}"#.as_slice(),
    ] {
        assert_eq!(
            parse_user_response(invalid, "warframe"),
            Err(HttpError::ProviderResponse)
        );
    }
}

#[test]
fn helix_subscription_response_matches_the_requested_type() {
    assert_eq!(
        parse_subscription_response(
            br#"{"data":[{"id":"subscription-1","status":"enabled","type":"channel.chat.message"}],"total":1,"max_total_cost":10,"total_cost":0}"#,
            ChatSubscription::Message,
        ),
        Ok(())
    );

    assert_eq!(
        parse_subscription_response(
            br#"{"data":[{"id":"subscription-1","status":"enabled","type":"channel.chat.clear"}]}"#,
            ChatSubscription::Message,
        ),
        Err(HttpError::ProviderResponse)
    );
}

#[test]
fn helix_send_response_is_authoritative_for_acceptance() {
    assert_eq!(
        parse_send_response(
            br#"{"data":[{"message_id":"message-1","is_sent":true,"drop_reason":null}]}"#
        ),
        Ok(ChatSendResult {
            message_id: Some("message-1".to_owned()),
            is_sent: true,
        })
    );
    assert_eq!(
        parse_send_response(
            br#"{"data":[{"message_id":"","is_sent":false,"drop_reason":{"code":"msg_duplicate","message":"duplicate"}}]}"#
        ),
        Ok(ChatSendResult {
            message_id: None,
            is_sent: false,
        })
    );

    for invalid in [
        br#"{"data":[]}"#.as_slice(),
        br#"{"data":[{"message_id":"","is_sent":true,"drop_reason":null}]}"#.as_slice(),
        br#"{"data":[{"message_id":"one","is_sent":true},{"message_id":"two","is_sent":true}]}"#
            .as_slice(),
    ] {
        assert_eq!(
            parse_send_response(invalid),
            Err(HttpError::ProviderResponse)
        );
    }
}
