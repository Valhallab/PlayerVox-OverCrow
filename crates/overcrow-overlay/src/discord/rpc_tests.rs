use super::{
    model::{DiscordRpcEvent, VoiceParticipant, VoiceSubscriptionEvent},
    rpc::{DISCORD_RPC_MESSAGE_MAX_BYTES, RpcParseError, parse_rpc_message},
};

fn voice_state(id: &str, nick: &str, speaking: bool) -> serde_json::Value {
    serde_json::json!({
        "nick": nick,
        "mute": false,
        "volume": 100,
        "voice_state": {
            "mute": false,
            "deaf": false,
            "self_mute": false,
            "self_deaf": false,
            "suppress": false
        },
        "user": {
            "id": id,
            "username": format!("user-{id}"),
            "global_name": format!("global-{id}"),
            "avatar": format!("avatar_{id}"),
            "bot": false
        },
        "speaking": speaking
    })
}

#[test]
fn selected_voice_channel_is_reduced_to_valid_display_state() {
    let payload = serde_json::json!({
        "cmd": "GET_SELECTED_VOICE_CHANNEL",
        "data": {
            "id": "123456789012345678",
            "name": "Squad voice",
            "type": 2,
            "guild_id": "999999999999999999",
            "voice_states": [
                voice_state("100", "Local pilot", false),
                voice_state("200", "Speaking pilot", true)
            ],
            "messages": [{"private": "ignored"}]
        },
        "evt": null,
        "nonce": "request-1"
    });

    let event = parse_rpc_message(payload.to_string().as_bytes()).unwrap();
    let DiscordRpcEvent::ChannelSnapshot {
        nonce,
        channel: Some(channel),
    } = event
    else {
        panic!("expected channel snapshot");
    };
    assert_eq!(nonce, "request-1");
    assert_eq!(channel.id, "123456789012345678");
    assert_eq!(channel.name, "Squad voice");
    assert_eq!(channel.participants.len(), 2);
    assert_eq!(channel.participants[0].display_name, "Local pilot");
    assert!(channel.participants[1].speaking);
}

#[test]
fn participant_uses_safe_name_fallbacks_and_combines_voice_flags() {
    let payload = serde_json::json!({
        "cmd": "DISPATCH",
        "data": {
            "nick": "",
            "mute": true,
            "voice_state": {
                "mute": false,
                "deaf": false,
                "self_mute": false,
                "self_deaf": true,
                "suppress": false
            },
            "user": {
                "id": "200",
                "username": "fallback-user",
                "global_name": "Global pilot",
                "avatar": null
            }
        },
        "evt": "VOICE_STATE_UPDATE",
        "nonce": null
    });

    let event = parse_rpc_message(payload.to_string().as_bytes()).unwrap();
    let DiscordRpcEvent::ParticipantUpdated(participant) = event else {
        panic!("expected participant update");
    };
    assert_eq!(participant.display_name, "Global pilot");
    assert!(participant.muted);
    assert!(participant.deafened);
    assert_eq!(participant.avatar_hash, None);
}

#[test]
fn parser_maps_authorization_authentication_and_dispatch_events() {
    let authorize =
        br#"{"cmd":"AUTHORIZE","data":{"code":"short-lived-code"},"evt":null,"nonce":"n1"}"#;
    let authenticated = br#"{"cmd":"AUTHENTICATE","data":{"user":{"id":"100","username":"pilot"},"expires":"2026-08-02T12:00:00Z"},"evt":null,"nonce":"n2"}"#;
    let speaking =
        br#"{"cmd":"DISPATCH","data":{"user_id":"200"},"evt":"SPEAKING_START","nonce":null}"#;
    let selected = br#"{"cmd":"DISPATCH","data":{"channel_id":null},"evt":"VOICE_CHANNEL_SELECT","nonce":null}"#;

    let DiscordRpcEvent::AuthorizationGranted { nonce, code } =
        parse_rpc_message(authorize).unwrap()
    else {
        panic!("expected authorization code");
    };
    assert_eq!(nonce, "n1");
    assert_eq!(code.expose(), "short-lived-code");
    assert!(!format!("{code:?}").contains("short-lived-code"));
    assert!(matches!(
        parse_rpc_message(authenticated).unwrap(),
        DiscordRpcEvent::Authenticated { ref nonce, ref user_id }
            if nonce == "n2" && user_id == "100"
    ));
    assert_eq!(
        parse_rpc_message(speaking).unwrap(),
        DiscordRpcEvent::SpeakingChanged {
            user_id: "200".to_owned(),
            speaking: true,
        }
    );
    assert_eq!(
        parse_rpc_message(selected).unwrap(),
        DiscordRpcEvent::ChannelSelected(None)
    );
}

#[test]
fn parser_rejects_uncorrelated_setup_responses() {
    assert_eq!(
        parse_rpc_message(br#"{"cmd":"AUTHORIZE","data":{"code":"code"}}"#),
        Err(RpcParseError::InvalidData)
    );
    assert_eq!(
        parse_rpc_message(
            br#"{"cmd":"AUTHENTICATE","data":{"user":{"id":"100","username":"pilot"}}}"#
        ),
        Err(RpcParseError::InvalidData)
    );
}

#[test]
fn parser_maps_ready_dispatch() {
    let ready = br#"{"cmd":"DISPATCH","data":{"v":1},"evt":"READY","nonce":null}"#;

    assert_eq!(parse_rpc_message(ready).unwrap(), DiscordRpcEvent::Ready);
}

#[test]
fn parser_maps_participant_creation() {
    let payload = serde_json::json!({
        "cmd": "DISPATCH",
        "data": voice_state("200", "New pilot", false),
        "evt": "VOICE_STATE_CREATE",
        "nonce": null
    });

    assert!(matches!(
        parse_rpc_message(payload.to_string().as_bytes()).unwrap(),
        DiscordRpcEvent::ParticipantCreated(ref participant)
            if participant.id == "200" && participant.display_name == "New pilot"
    ));
}

#[test]
fn parser_maps_speaking_stop() {
    let stopped =
        br#"{"cmd":"DISPATCH","data":{"user_id":"200"},"evt":"SPEAKING_STOP","nonce":null}"#;

    assert_eq!(
        parse_rpc_message(stopped).unwrap(),
        DiscordRpcEvent::SpeakingChanged {
            user_id: "200".to_owned(),
            speaking: false,
        }
    );
}

#[test]
fn parser_maps_voice_deletion_and_provider_errors_without_messages() {
    let deleted = br#"{"cmd":"DISPATCH","data":{"user":{"id":"200"}},"evt":"VOICE_STATE_DELETE","nonce":null}"#;
    let error = br#"{"cmd":"AUTHENTICATE","data":{"code":4009,"message":"private provider detail"},"evt":"ERROR","nonce":"n"}"#;

    assert_eq!(
        parse_rpc_message(deleted).unwrap(),
        DiscordRpcEvent::ParticipantDeleted {
            user_id: "200".to_owned()
        }
    );
    let parsed = parse_rpc_message(error).unwrap();
    assert_eq!(
        parsed,
        DiscordRpcEvent::ProviderError {
            command: Some("AUTHENTICATE".to_owned()),
            code: 4009,
            nonce: Some("n".to_owned()),
        }
    );
    assert!(!format!("{parsed:?}").contains("private provider detail"));
}

#[test]
fn parser_maps_only_known_subscription_acknowledgements() {
    let unsubscribed = br#"{"cmd":"UNSUBSCRIBE","data":{"evt":"SPEAKING_START"},"nonce":"n"}"#;
    let channel_select =
        br#"{"cmd":"SUBSCRIBE","data":{"evt":"VOICE_CHANNEL_SELECT"},"nonce":"n"}"#;

    assert_eq!(
        parse_rpc_message(unsubscribed).unwrap(),
        DiscordRpcEvent::SubscriptionChanged {
            subscribed: false,
            event: VoiceSubscriptionEvent::SpeakingStart,
            nonce: "n".to_owned(),
        }
    );
    assert_eq!(
        parse_rpc_message(channel_select).unwrap(),
        DiscordRpcEvent::SubscriptionChanged {
            subscribed: true,
            event: VoiceSubscriptionEvent::VoiceChannelSelect,
            nonce: "n".to_owned(),
        }
    );
    assert_eq!(
        parse_rpc_message(br#"{"cmd":"SUBSCRIBE","data":{"evt":"MESSAGE_CREATE"},"nonce":"n"}"#),
        Err(RpcParseError::InvalidData)
    );
}

#[test]
fn parser_rejects_oversized_malformed_and_unbounded_payloads() {
    assert_eq!(
        parse_rpc_message(&vec![b' '; DISCORD_RPC_MESSAGE_MAX_BYTES + 1]),
        Err(RpcParseError::Oversized)
    );
    assert_eq!(parse_rpc_message(b"{"), Err(RpcParseError::Malformed));
    let oversized_event = serde_json::json!({
        "cmd": "DISPATCH",
        "evt": "X".repeat(65),
        "data": {}
    });
    assert_eq!(
        parse_rpc_message(oversized_event.to_string().as_bytes()),
        Err(RpcParseError::InvalidData)
    );
    let non_ascii_event = serde_json::json!({
        "cmd": "DISPATCH",
        "evt": "SPEAKING_É",
        "data": {}
    });
    assert_eq!(
        parse_rpc_message(non_ascii_event.to_string().as_bytes()),
        Err(RpcParseError::InvalidData)
    );

    let mut oversized_channel = serde_json::json!({
        "cmd": "GET_SELECTED_VOICE_CHANNEL",
        "data": {"id": "1", "name": "x", "voice_states": []},
        "evt": null
    });
    oversized_channel["data"]["name"] = serde_json::Value::String("x".repeat(129));
    assert_eq!(
        parse_rpc_message(oversized_channel.to_string().as_bytes()),
        Err(RpcParseError::InvalidData)
    );

    let participants = (0..65)
        .map(|index| voice_state(&index.to_string(), "pilot", false))
        .collect::<Vec<_>>();
    let too_many = serde_json::json!({
        "cmd": "GET_SELECTED_VOICE_CHANNEL",
        "data": {"id": "1", "name": "voice", "voice_states": participants},
        "evt": null
    });
    assert_eq!(
        parse_rpc_message(too_many.to_string().as_bytes()),
        Err(RpcParseError::InvalidData)
    );
}

#[test]
fn participant_sorting_keeps_local_then_speakers_then_names() {
    let mut participants = vec![
        VoiceParticipant::for_test("3", "Zulu", false),
        VoiceParticipant::for_test("2", "alpha", true),
        VoiceParticipant::for_test("1", "Local", false),
        VoiceParticipant::for_test("4", "Bravo", true),
    ];

    super::model::sort_participants(&mut participants, Some("1"));

    assert_eq!(
        participants
            .iter()
            .map(|participant| participant.id.as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "4", "3"]
    );
}
