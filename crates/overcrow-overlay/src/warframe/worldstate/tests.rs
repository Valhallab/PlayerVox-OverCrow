use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use overcrow_config::{
    FissureEra, WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS, WarframePrefs, WarframePrefsStore,
};
use serde_json::{Map, Value, json};

use super::{
    client::publish_if_changed,
    parse::{SYNDICATE_MISSION_INPUT_MAX, filter_fissures, filter_invasions, parse_worldstate},
};
use crate::runtime::latest_channel;
use crate::warframe::{
    format_mission_type, format_node, invasion_done_key,
    model::{
        ACTIVITY_MISSION_MAX, CYCLE_LIST_MAX, ERROR_MAX_CHARS, FISSURE_LIST_MAX, INVASION_LIST_MAX,
        STRING_MAX_CHARS, WorldstateSnapshot,
    },
};

const FIXTURE: &[u8] = include_bytes!("fixtures/minimal.json");

fn activity_rows() -> Vec<Value> {
    (0..ACTIVITY_MISSION_MAX)
        .map(|index| {
            json!({
                "missionType": format!("Mission{index}"),
                "modifierType": format!("Modifier{index}"),
                "node": format!("SolNode{index}"),
            })
        })
        .collect()
}

#[test]
fn unchanged_worldstate_frame_does_not_publish_or_replace_the_snapshot() {
    let initial = WorldstateSnapshot::default();
    let (publisher, receiver) = latest_channel(initial.clone());
    let mut published = publisher.current().value;
    let repaints = AtomicUsize::new(0);
    let next = WorldstateSnapshot {
        error: Some("offline".to_owned()),
        ..initial
    };

    assert!(publish_if_changed(
        &publisher,
        &mut published,
        next.clone(),
        &|| {
            repaints.fetch_add(1, Ordering::SeqCst);
        },
    ));
    let first = receiver.take_latest().expect("changed frame is ready");
    let retained = Arc::clone(&first.value);

    assert!(!publish_if_changed(
        &publisher,
        &mut published,
        next,
        &|| {
            repaints.fetch_add(1, Ordering::SeqCst);
        },
    ));
    assert!(receiver.take_latest().is_none());
    assert!(Arc::ptr_eq(&retained, &publisher.current().value));
    assert_eq!(repaints.load(Ordering::SeqCst), 1);
}

#[test]
fn parses_cycles_baro_and_fissures_from_fixture() {
    let snapshot = parse_worldstate(FIXTURE, 1_784_309_062).unwrap();
    assert!(snapshot.server_time_secs > 0);
    assert_eq!(snapshot.fetched_at_secs, 1_784_309_062);
    assert_eq!(snapshot.last_attempt_at_secs, 1_784_309_062);
    assert!(!snapshot.cycles.is_empty());
    assert!(snapshot.cycles.iter().any(|cycle| cycle.label == "Cetus"));
    assert!(snapshot.cycles.iter().any(|cycle| cycle.label == "Vallis"));
    assert!(snapshot.cycles.iter().any(|cycle| cycle.label == "Cambion"));
    assert!(snapshot.baro.is_some());
    assert!(!snapshot.fissures.is_empty());
}

#[test]
fn cycles_use_day_night_not_raw_bounty_labels() {
    let snapshot = parse_worldstate(FIXTURE, 1_784_309_062).unwrap();
    let cetus = snapshot
        .cycles
        .iter()
        .find(|cycle| cycle.id == "cetus")
        .expect("cetus");
    assert!(
        matches!(cetus.state.as_deref(), Some("day" | "night")),
        "unexpected cetus state {:?}",
        cetus.state
    );
    let vallis = snapshot
        .cycles
        .iter()
        .find(|cycle| cycle.id == "vallis")
        .expect("vallis");
    assert!(matches!(vallis.state.as_deref(), Some("warm" | "cold")));
}

#[test]
fn daily_reset_is_utc_midnight_not_sortie() {
    let snapshot = parse_worldstate(FIXTURE, 1_784_309_062).unwrap();
    let reset = snapshot.daily_reset_at_secs.expect("daily reset");
    assert_eq!(reset % 86_400, 0);
    assert!(reset > snapshot.server_time_secs);
}

#[test]
fn fixture_nodes_and_missions_have_readable_labels() {
    let snapshot = parse_worldstate(FIXTURE, 1_784_309_062).unwrap();
    let fissure = snapshot
        .fissures
        .iter()
        .find(|fissure| fissure.node == "SolNode17")
        .expect("fixture contains SolNode17");
    assert_eq!(format_node(&fissure.node), "Proteus · Neptune");
    assert_eq!(format_mission_type(&fissure.mission_type), "Defense");

    let baro = snapshot.baro.as_ref().expect("baro present");
    let location = baro.location.as_deref().expect("baro location");
    assert!(
        location.contains('·') || location.contains("Relay") || location.contains("TennoCon"),
        "unexpected baro location: {location}"
    );
}

#[test]
fn rejects_invalid_json() {
    assert!(parse_worldstate(b"not-json", 0).is_err());
}

#[test]
fn filter_respects_era_and_source_flags() {
    let snapshot = parse_worldstate(FIXTURE, 1_784_309_062).unwrap();
    let prefs = WarframePrefs {
        fissure_eras: vec![FissureEra::Axi],
        show_normal: false,
        show_railjack: false,
        show_steel_path: true,
        ..WarframePrefs::default()
    };
    let filtered = filter_fissures(&snapshot.fissures, &prefs, snapshot.server_time_secs);
    assert!(
        filtered.iter().all(|fissure| fissure.era == FissureEra::Axi
            && fissure.steel_path
            && !fissure.is_storm)
    );
}

#[test]
fn void_storms_are_included_as_railjack_fissures() {
    let snapshot = parse_worldstate(FIXTURE, 1_784_309_062).unwrap();
    assert!(
        snapshot.fissures.iter().any(|fissure| fissure.is_storm),
        "expected VoidStorms in the fissure list"
    );
    // Star-chart + storms should exceed ActiveMissions alone (8 in fixture).
    assert!(snapshot.fissures.len() > 8);
}

#[test]
fn parses_sortie_archon_and_invasions() {
    // Within fixture Sortie window (expires 1784390400) and Archon window.
    let snapshot = parse_worldstate(FIXTURE, 1_784_309_062).unwrap();
    let sortie = snapshot.sortie.expect("sortie");
    assert!(
        sortie.boss.contains("Lephantis")
            || sortie.boss.contains("lephantis")
            || !sortie.boss.is_empty()
    );
    assert_eq!(sortie.missions.len(), 3);
    assert!(sortie.missions[0].modifier.is_some());

    let archon = snapshot.archon.expect("archon");
    assert!(archon.boss.to_ascii_lowercase().contains("nira"));
    assert_eq!(archon.missions.len(), 3);

    assert!(snapshot.invasions.len() >= 2);
    assert_eq!(
        snapshot.invasions[0].instance_id,
        "6a5a60000000000000000001"
    );
    assert!(
        snapshot
            .invasions
            .iter()
            .any(|inv| inv.attacker_reward.is_some() || inv.defender_reward.is_some())
    );
}

#[test]
fn current_syndicate_volume_does_not_hide_dedicated_missions() {
    let mut syndicates = (0..37)
        .map(|index| {
            json!({
                "Tag": format!("UnusedSyndicate{index}"),
                "Expiry": 2_000,
            })
        })
        .collect::<Vec<_>>();
    syndicates[32] = json!({"Tag": "CetusSyndicate", "Expiry": 2_000});
    syndicates[33] = json!({"Tag": "ZarimanSyndicate", "Expiry": 2_000});

    let worldstate = json!({
        "SyndicateMissions": syndicates,
        "ActiveMissions": [{
            "Modifier": "NightmareMode",
            "MissionType": "Nightmare",
            "Node": "SolNode1",
            "Expiry": 2_000,
        }],
        "Sorties": [{
            "Boss": "SORTIE_BOSS_LEPHANTIS",
            "Expiry": 2_000,
            "Variants": activity_rows(),
        }],
        "LiteSorties": [{
            "Boss": "ArchonNira",
            "Expiry": 2_000,
            "Missions": activity_rows(),
        }],
        "Invasions": [{
            "_id": {"$oid": "provider-invasion"},
            "Node": "SolNode2",
            "Faction": "FC_GRINEER",
            "DefenderFaction": "FC_CORPUS",
            "Count": 10,
            "Goal": 100,
        }],
    });

    let snapshot = parse_worldstate(
        &serde_json::to_vec(&worldstate).expect("serialize current provider shape"),
        1_000,
    )
    .expect("unrelated syndicates must not reject dedicated missions");

    assert_eq!(snapshot.sortie.as_ref().unwrap().missions.len(), 3);
    assert_eq!(snapshot.archon.as_ref().unwrap().missions.len(), 3);
    assert_eq!(snapshot.invasions.len(), 1);
    assert_eq!(snapshot.invasions[0].instance_id, "provider-invasion");
    assert!(snapshot.fissures.is_empty());
    assert!(snapshot.cycles.iter().any(|cycle| cycle.id == "cetus"));
    assert!(snapshot.cycles.iter().any(|cycle| cycle.id == "zariman"));
}

#[test]
fn invasion_filter_hides_completed_and_prioritizes_watchlist() {
    let snapshot = parse_worldstate(FIXTURE, 1_784_309_062).unwrap();
    let mut prefs = WarframePrefs {
        invasion_hide_completed: true,
        ..WarframePrefs::default()
    };
    let filtered = filter_invasions(&snapshot.invasions, &prefs);
    assert!(filtered.iter().all(|inv| !inv.completed));
    assert!(!filtered.is_empty());

    prefs.invasion_reward_watchlist = vec!["EnergyComponent".to_owned()];
    let ranked = filter_invasions(&snapshot.invasions, &prefs);
    assert_eq!(
        ranked[0]
            .attacker_reward
            .as_ref()
            .map(|r| r.item_key.as_str()),
        Some("EnergyComponent")
    );
}

fn assert_sequence_overflow(field: &str, entry: Value, max: usize) {
    let mut root = Map::new();
    root.insert(field.to_owned(), Value::Array(vec![entry; max + 1]));
    let bytes = serde_json::to_vec(&root).expect("serialize adversarial worldstate");

    let error = parse_worldstate(&bytes, 1_000).expect_err("max + 1 entries must be rejected");
    assert!(
        error.0.contains("too many entries"),
        "unexpected parse error: {}",
        error.0
    );
    assert!(error.0.chars().count() <= ERROR_MAX_CHARS);
}

#[test]
fn rejects_oversized_generic_top_level_provider_sequences() {
    for (field, entry, max) in [
        ("VoidTraders", json!({}), CYCLE_LIST_MAX),
        ("ActiveMissions", json!({}), FISSURE_LIST_MAX),
        ("VoidStorms", json!({}), FISSURE_LIST_MAX),
        ("Sorties", json!({}), ACTIVITY_MISSION_MAX),
        ("LiteSorties", json!({}), ACTIVITY_MISSION_MAX),
        ("Invasions", json!({}), INVASION_LIST_MAX),
    ] {
        assert_sequence_overflow(field, entry, max);
    }
}

#[test]
fn rejects_syndicate_input_beyond_the_dedicated_scan_bound() {
    assert_sequence_overflow(
        "SyndicateMissions",
        json!({"Tag": "UnusedSyndicate", "Expiry": 2_000}),
        SYNDICATE_MISSION_INPUT_MAX,
    );
}

#[test]
fn accepts_the_syndicate_scan_limit_and_keeps_the_first_cycle_row() {
    let mut syndicates = (0..SYNDICATE_MISSION_INPUT_MAX)
        .map(|index| json!({"Tag": format!("UnusedSyndicate{index}"), "Expiry": 2_000}))
        .collect::<Vec<_>>();
    syndicates[0] = json!({"Tag": "CetusSyndicate"});
    syndicates[1] = json!({"Tag": "CetusSyndicate", "Expiry": 2_000});
    syndicates[SYNDICATE_MISSION_INPUT_MAX - 1] =
        json!({"Tag": "ZarimanSyndicate", "Expiry": 2_000});

    let worldstate = json!({"SyndicateMissions": syndicates});
    let snapshot = parse_worldstate(
        &serde_json::to_vec(&worldstate).expect("serialize boundary worldstate"),
        1_000,
    )
    .expect("the exact scan limit must be accepted");

    assert!(!snapshot.cycles.iter().any(|cycle| cycle.id == "cetus"));
    assert!(snapshot.cycles.iter().any(|cycle| cycle.id == "zariman"));
}

#[test]
fn rejects_oversized_nested_activity_mission_sequences() {
    let sortie = json!({
        "Sorties": [{
            "Expiry": 2_000,
            "Variants": vec![json!({}); ACTIVITY_MISSION_MAX + 1],
        }],
    });
    let error = parse_worldstate(
        &serde_json::to_vec(&sortie).expect("serialize sortie"),
        1_000,
    )
    .expect_err("oversized sortie mission list must fail");
    assert!(error.0.contains("too many entries"));
    assert!(error.0.chars().count() <= ERROR_MAX_CHARS);

    let archon = json!({
        "LiteSorties": [{
            "Expiry": 2_000,
            "Missions": vec![json!({}); ACTIVITY_MISSION_MAX + 1],
        }],
    });
    let error = parse_worldstate(
        &serde_json::to_vec(&archon).expect("serialize archon hunt"),
        1_000,
    )
    .expect_err("oversized archon mission list must fail");
    assert!(error.0.contains("too many entries"));
    assert!(error.0.chars().count() <= ERROR_MAX_CHARS);
}

#[test]
fn rejects_oversized_nested_invasion_reward_sequence() {
    let worldstate = json!({
        "Invasions": [{
            "Node": "SolNode1",
            "AttackerReward": {
                "countedItems": vec![json!({
                    "ItemType": "/Lotus/Types/Items/Research/EnergyComponent",
                    "ItemCount": 1,
                }); INVASION_LIST_MAX + 1],
            },
        }],
    });
    let error = parse_worldstate(
        &serde_json::to_vec(&worldstate).expect("serialize invasion"),
        1_000,
    )
    .expect_err("oversized invasion reward list must fail");
    assert!(error.0.contains("too many entries"));
    assert!(error.0.chars().count() <= ERROR_MAX_CHARS);
}

#[test]
fn provider_strings_are_bounded_in_the_published_snapshot() {
    let long = "x".repeat(STRING_MAX_CHARS + 64);
    let worldstate = json!({
        "VoidTraders": [{
            "Character": "Baro'Ki Teel",
            "Node": long,
            "Activation": 900,
            "Expiry": 2_000,
        }],
        "ActiveMissions": [{
            "Modifier": "VoidT1",
            "MissionType": long,
            "Node": long,
            "Expiry": 2_000,
        }],
        "Sorties": [{
            "Boss": long,
            "Expiry": 2_000,
            "Variants": [{
                "missionType": long,
                "modifierType": long,
                "node": long,
            }],
        }],
        "LiteSorties": [{
            "Boss": long,
            "Expiry": 2_000,
            "Missions": [{"missionType": long, "node": long}],
        }],
        "Invasions": [{
            "Node": long,
            "Faction": long,
            "DefenderFaction": long,
            "AttackerReward": {"countedItems": [{"ItemType": long}]},
        }],
    });
    let snapshot = parse_worldstate(
        &serde_json::to_vec(&worldstate).expect("serialize long strings"),
        1_000,
    )
    .expect("bounded strings remain valid");

    let mut strings = Vec::new();
    strings.extend(snapshot.baro.and_then(|baro| baro.location));
    for fissure in snapshot.fissures {
        strings.extend([fissure.mission_type, fissure.node]);
    }
    for activity in [
        snapshot.sortie.map(|sortie| (sortie.boss, sortie.missions)),
        snapshot.archon.map(|archon| (archon.boss, archon.missions)),
    ]
    .into_iter()
    .flatten()
    {
        strings.push(activity.0);
        for mission in activity.1 {
            strings.extend([mission.mission_type, mission.node]);
            strings.extend(mission.modifier);
        }
    }
    for invasion in snapshot.invasions {
        strings.extend([
            invasion.node,
            invasion.attacker_faction,
            invasion.defender_faction,
        ]);
        for reward in [invasion.attacker_reward, invasion.defender_reward]
            .into_iter()
            .flatten()
        {
            strings.extend([reward.item_key, reward.label]);
        }
    }
    assert!(!strings.is_empty());
    assert!(
        strings
            .iter()
            .all(|value| value.chars().count() <= STRING_MAX_CHARS),
        "unbounded output: {strings:?}"
    );
}

fn parse_baro_candidates(entries: Vec<Value>, now_secs: u64) -> crate::warframe::model::BaroStatus {
    let worldstate = json!({"VoidTraders": entries});
    parse_worldstate(
        &serde_json::to_vec(&worldstate).expect("serialize Baro candidates"),
        now_secs,
    )
    .expect("parse Baro candidates")
    .baro
    .expect("select a Baro candidate")
}

fn baro(node: &str, activation_secs: u64, expiry_secs: u64) -> Value {
    json!({
        "Character": "Baro'Ki Teel",
        "Node": node,
        "Activation": activation_secs,
        "Expiry": expiry_secs,
    })
}

#[test]
fn expired_baro_entries_cannot_outrank_a_future_visit() {
    let selected = parse_baro_candidates(
        vec![
            baro("MercuryHUB", 100, 1_000),
            baro("VenusHUB", 1_100, 2_000),
        ],
        1_000,
    );

    assert!(!selected.present);
    assert_eq!(selected.activation_secs, 1_100);
    assert_eq!(selected.expiry_secs, 2_000);
}

#[test]
fn present_baro_visit_outranks_future_visits() {
    let selected = parse_baro_candidates(
        vec![
            baro("VenusHUB", 1_100, 2_000),
            baro("MercuryHUB", 900, 1_200),
        ],
        1_000,
    );

    assert!(selected.present);
    assert_eq!(selected.activation_secs, 900);
}

#[test]
fn future_baro_visits_are_ranked_by_earliest_activation_before_known_location() {
    let selected = parse_baro_candidates(
        vec![
            baro("MercuryHUB", 1_200, 2_000),
            baro("UnmappedHUB", 1_100, 2_000),
        ],
        1_000,
    );

    assert!(!selected.present);
    assert_eq!(selected.activation_secs, 1_100);
    assert_eq!(selected.location.as_deref(), Some("UnmappedHUB"));
}

fn invasion_with_id(id: Option<&str>) -> Value {
    let mut invasion = json!({
        "Node": "SolNode1",
        "Faction": "FC_GRINEER",
        "DefenderFaction": "FC_CORPUS",
        "Goal": 42_000,
        "AttackerReward": {"countedItems": [{"ItemType": "ResourceA"}]},
        "DefenderReward": {"countedItems": [{"ItemType": "ResourceB"}]},
    });
    if let Some(id) = id {
        invasion
            .as_object_mut()
            .expect("invasion object")
            .insert("_id".to_owned(), json!({"$oid": id}));
    }
    invasion
}

fn invasion_with_raw_id(id: Value) -> Value {
    let mut invasion = invasion_with_id(None);
    invasion
        .as_object_mut()
        .expect("invasion object")
        .insert("_id".to_owned(), id);
    invasion
}

#[test]
fn same_node_invasions_use_distinct_provider_instance_ids() {
    let worldstate = json!({
        "Invasions": [
            invasion_with_id(Some("provider-object-a")),
            invasion_with_id(Some("provider-object-b")),
        ],
    });
    let snapshot = parse_worldstate(
        &serde_json::to_vec(&worldstate).expect("serialize invasions"),
        1_000,
    )
    .expect("parse invasions");

    assert_eq!(snapshot.invasions[0].instance_id, "provider-object-a");
    assert_eq!(snapshot.invasions[1].instance_id, "provider-object-b");
    assert_ne!(
        invasion_done_key(&snapshot.invasions[0].instance_id),
        invasion_done_key(&snapshot.invasions[1].instance_id)
    );
}

fn round_trip_invasion_completion_key(provider_id: &str) -> String {
    let worldstate = json!({"Invasions": [invasion_with_id(Some(provider_id))]});
    let instance_id = parse_worldstate(
        &serde_json::to_vec(&worldstate).expect("serialize invasion"),
        1_000,
    )
    .expect("parse invasion")
    .invasions
    .remove(0)
    .instance_id;
    let done_key = invasion_done_key(&instance_id);
    assert!(
        done_key.chars().count() <= WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS,
        "completion key exceeded persisted preference bound: {}",
        done_key.chars().count()
    );

    let mut prefs = WarframePrefs::default();
    prefs.toggle_activity_done(&done_key);
    assert!(prefs.activity_is_done(&done_key));
    assert!(prefs.clone().validate().is_ok());

    let temp = tempfile::tempdir().expect("temporary preference directory");
    let store = WarframePrefsStore::from_path(temp.path().join("warframe.json"));
    store.save(&prefs).expect("save completion preference");
    let loaded = store.load();
    assert!(loaded.warning.is_none());
    assert!(loaded.prefs.activity_is_done(&done_key));

    instance_id
}

#[test]
fn provider_invasion_id_at_persisted_key_boundary_round_trips() {
    let provider_id_max = WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS - "invasion:".chars().count();
    let provider_id = "a".repeat(provider_id_max);

    assert_eq!(
        round_trip_invasion_completion_key(&provider_id),
        provider_id
    );
}

#[test]
fn provider_invasion_id_above_persisted_key_boundary_uses_roundtrippable_fallback() {
    let provider_id_max = WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS - "invasion:".chars().count();
    let provider_id = "a".repeat(provider_id_max + 1);
    let instance_id = round_trip_invasion_completion_key(&provider_id);

    assert_ne!(instance_id, provider_id);
    assert_eq!(instance_id.len(), 16);
}

#[test]
fn invasion_without_provider_id_has_a_deterministic_hex_fallback() {
    let first = parse_worldstate(FIXTURE, 1_000)
        .expect("first parse")
        .invasions
        .into_iter()
        .find(|invasion| invasion.node == "SolNode41")
        .expect("fixture invasion without provider id")
        .instance_id;
    let second = parse_worldstate(FIXTURE, 2_000)
        .expect("second parse")
        .invasions
        .into_iter()
        .find(|invasion| invasion.node == "SolNode41")
        .expect("fixture invasion without provider id")
        .instance_id;

    assert_eq!(first, second);
    assert_eq!(first.len(), 16);
    assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[test]
fn malformed_provider_id_shapes_are_ignored_for_deterministic_fallback() {
    let without_id = json!({"Invasions": [invasion_with_id(None)]});
    let expected = parse_worldstate(
        &serde_json::to_vec(&without_id).expect("serialize no-id invasion"),
        1_000,
    )
    .expect("parse no-id invasion")
    .invasions
    .remove(0)
    .instance_id;
    let malformed_ids = [
        json!(42),
        json!("scalar-not-object"),
        json!([{"$oid": "nested-in-array"}]),
        json!({"unexpected": ["object", "shape"]}),
        json!({"$oid": 42}),
        json!({"$oid": ["non-string"]}),
        json!({"$oid": {"nested": "non-string"}}),
    ];

    for (index, malformed_id) in malformed_ids.into_iter().enumerate() {
        let worldstate = json!({"Invasions": [invasion_with_raw_id(malformed_id)]});
        let bytes = serde_json::to_vec(&worldstate).expect("serialize malformed-id invasion");
        let first = parse_worldstate(&bytes, 1_000)
            .unwrap_or_else(|error| panic!("shape {index} rejected worldstate: {error}"))
            .invasions
            .remove(0)
            .instance_id;
        let second = parse_worldstate(&bytes, 2_000)
            .unwrap_or_else(|error| panic!("shape {index} rejected second parse: {error}"))
            .invasions
            .remove(0)
            .instance_id;

        assert_eq!(first, expected, "shape {index} was not ignored");
        assert_eq!(second, expected, "shape {index} was not deterministic");
    }
}

#[test]
fn provider_invasion_id_is_sanitized_to_bounded_ascii() {
    let unsafe_id = format!(" id/é\n{}", "Z".repeat(STRING_MAX_CHARS + 32));
    let worldstate = json!({"Invasions": [invasion_with_id(Some(&unsafe_id))]});
    let instance_id = parse_worldstate(
        &serde_json::to_vec(&worldstate).expect("serialize invasion"),
        1_000,
    )
    .expect("parse invasion")
    .invasions
    .remove(0)
    .instance_id;

    assert!(!instance_id.is_empty());
    assert!(instance_id.is_ascii());
    assert!(instance_id.chars().count() <= STRING_MAX_CHARS);
    assert!(
        instance_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    );
}

#[test]
fn distinct_invalid_provider_ids_use_distinct_deterministic_fallbacks() {
    let worldstate = json!({
        "Invasions": [
            invasion_with_id(Some("unsafe/id")),
            invasion_with_id(Some("unsafe?id")),
        ],
    });
    let invasions = parse_worldstate(
        &serde_json::to_vec(&worldstate).expect("serialize invasions"),
        1_000,
    )
    .expect("parse invasions")
    .invasions;

    assert_ne!(invasions[0].instance_id, invasions[1].instance_id);
    for invasion in invasions {
        assert_eq!(invasion.instance_id.len(), 16);
        assert!(
            invasion
                .instance_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
    }
}

#[test]
fn overlong_provider_ids_with_the_same_first_95_chars_do_not_collide() {
    let shared_prefix = "a".repeat(95);
    let first_id = format!("{shared_prefix}first-tail");
    let second_id = format!("{shared_prefix}second-tail");
    let worldstate = json!({
        "Invasions": [
            invasion_with_id(Some(&first_id)),
            invasion_with_id(Some(&second_id)),
        ],
    });
    let bytes = serde_json::to_vec(&worldstate).expect("serialize invasions");

    let first_parse = parse_worldstate(&bytes, 1_000)
        .expect("first parse")
        .invasions;
    let second_parse = parse_worldstate(&bytes, 2_000)
        .expect("second parse")
        .invasions;

    assert_ne!(first_parse[0].instance_id, first_parse[1].instance_id);
    assert_eq!(first_parse[0].instance_id, second_parse[0].instance_id);
    assert_eq!(first_parse[1].instance_id, second_parse[1].instance_id);
}
