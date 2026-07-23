//! Worldstate parsing aligned with community-validated cycle math (WFCD).
//!
//! Open-world timers are **not** the raw syndicate bounty windows. Cetus/Cambion
//! derive day/night (fass/vome) from the Ostron bounty expiry; Vallis is a fixed
//! epoch loop; Zariman uses the Zariman bounty window for remaining time and
//! faction state. Fissures are `ActiveMissions` plus Railjack `VoidStorms`.

use overcrow_config::{FissureEra, WarframePrefs};
use serde::{
    Deserialize, Deserializer,
    de::{Error as _, IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;

use super::super::activity_keys::invasion_instance_id_max_chars;
use super::super::bounded_serde::deserialize_capped_vec;
use super::super::labels::{format_boss, format_faction, format_item, format_modifier, item_key};
use super::super::locations::{format_node, resolve_node};
use super::super::model::{
    ACTIVITY_MISSION_MAX, ActivityMission, ArchonHunt, BaroStatus, CYCLE_LIST_MAX, CycleStatus,
    ERROR_MAX_CHARS, FISSURE_LIST_MAX, FissureMission, INVASION_LIST_MAX, InvasionMission,
    RewardLine, STRING_MAX_CHARS, SortieMission, WorldstateSnapshot, bound_chars,
};

#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

#[derive(Deserialize)]
struct RawWorldstate {
    #[serde(rename = "Time", default)]
    time: Option<u64>,
    #[serde(
        rename = "SyndicateMissions",
        default,
        deserialize_with = "deserialize_cycle_syndicate_missions"
    )]
    syndicate_missions: Vec<RawSyndicateMission>,
    #[serde(
        rename = "VoidTraders",
        default,
        deserialize_with = "deserialize_void_traders"
    )]
    void_traders: Vec<RawVoidTrader>,
    #[serde(
        rename = "ActiveMissions",
        default,
        deserialize_with = "deserialize_active_missions"
    )]
    active_missions: Vec<RawActiveMission>,
    #[serde(
        rename = "VoidStorms",
        default,
        deserialize_with = "deserialize_void_storms"
    )]
    void_storms: Vec<RawVoidStorm>,
    #[serde(rename = "Sorties", default, deserialize_with = "deserialize_sorties")]
    sorties: Vec<RawSortie>,
    #[serde(
        rename = "LiteSorties",
        default,
        deserialize_with = "deserialize_lite_sorties"
    )]
    lite_sorties: Vec<RawLiteSortie>,
    #[serde(
        rename = "Invasions",
        default,
        deserialize_with = "deserialize_invasions"
    )]
    invasions: Vec<RawInvasion>,
}

#[derive(Deserialize)]
struct RawSyndicateMission {
    #[serde(rename = "Tag", default)]
    tag: Option<String>,
    #[serde(rename = "Expiry")]
    expiry: Option<Value>,
}

#[derive(Deserialize)]
struct RawVoidTrader {
    #[serde(rename = "Character", default)]
    character: Option<String>,
    #[serde(rename = "Node", default)]
    node: Option<String>,
    #[serde(rename = "Activation")]
    activation: Option<Value>,
    #[serde(rename = "Expiry")]
    expiry: Option<Value>,
}

#[derive(Deserialize)]
struct RawActiveMission {
    #[serde(rename = "Modifier", default)]
    modifier: Option<String>,
    #[serde(rename = "MissionType", default)]
    mission_type: Option<String>,
    #[serde(rename = "Node", default)]
    node: Option<String>,
    #[serde(rename = "Expiry")]
    expiry: Option<Value>,
    #[serde(rename = "Hard", default)]
    hard: bool,
}

#[derive(Deserialize)]
struct RawVoidStorm {
    #[serde(rename = "ActiveMissionTier", default)]
    active_mission_tiers: Option<String>,
    #[serde(rename = "Node", default)]
    node: Option<String>,
    #[serde(rename = "Expiry")]
    expiry: Option<Value>,
}

#[derive(Deserialize)]
struct RawSortie {
    #[serde(rename = "Boss", default)]
    boss: Option<String>,
    #[serde(rename = "Expiry")]
    expiry: Option<Value>,
    #[serde(
        rename = "Variants",
        default,
        deserialize_with = "deserialize_sortie_variants"
    )]
    variants: Vec<RawSortieVariant>,
}

#[derive(Deserialize)]
struct RawSortieVariant {
    #[serde(rename = "missionType", default)]
    mission_type: Option<String>,
    #[serde(rename = "modifierType", default)]
    modifier_type: Option<String>,
    #[serde(rename = "node", default)]
    node: Option<String>,
}

#[derive(Deserialize)]
struct RawLiteSortie {
    #[serde(rename = "Boss", default)]
    boss: Option<String>,
    #[serde(rename = "Expiry")]
    expiry: Option<Value>,
    #[serde(
        rename = "Missions",
        default,
        deserialize_with = "deserialize_archon_missions"
    )]
    missions: Vec<RawArchonMission>,
}

#[derive(Deserialize)]
struct RawArchonMission {
    #[serde(rename = "missionType", default)]
    mission_type: Option<String>,
    #[serde(rename = "node", default)]
    node: Option<String>,
}

#[derive(Deserialize)]
struct RawInvasion {
    #[serde(rename = "_id", default)]
    id: Option<RawObjectId>,
    #[serde(rename = "Node", default)]
    node: Option<String>,
    #[serde(rename = "Faction", default)]
    faction: Option<String>,
    #[serde(rename = "DefenderFaction", default)]
    defender_faction: Option<String>,
    #[serde(rename = "Count", default)]
    count: Option<i64>,
    #[serde(rename = "Goal", default)]
    goal: Option<i64>,
    #[serde(rename = "Completed", default)]
    completed: bool,
    #[serde(rename = "AttackerReward", default)]
    attacker_reward: RawReward,
    #[serde(rename = "DefenderReward", default)]
    defender_reward: RawReward,
}

#[derive(Default)]
struct RawObjectId {
    oid: Option<String>,
    invalid_oid_digest: Option<u64>,
}

#[derive(Deserialize)]
#[serde(field_identifier)]
enum RawObjectIdField {
    #[serde(rename = "$oid")]
    Oid,
    #[serde(other)]
    Other,
}

#[derive(Default)]
struct LenientProviderOid {
    value: Option<String>,
    invalid_digest: Option<u64>,
}

impl LenientProviderOid {
    fn from_borrowed(value: &str) -> Self {
        if let Some(value) = sanitize_invasion_instance_id(value) {
            Self {
                value: Some(value),
                invalid_digest: None,
            }
        } else {
            Self {
                value: None,
                invalid_digest: Some(fnv1a_digest(value.as_bytes())),
            }
        }
    }

    fn from_owned(value: String) -> Self {
        if sanitize_invasion_instance_id(&value).is_some() {
            Self {
                value: Some(value),
                invalid_digest: None,
            }
        } else {
            Self {
                value: None,
                invalid_digest: Some(fnv1a_digest(value.as_bytes())),
            }
        }
    }
}

impl<'de> Deserialize<'de> for LenientProviderOid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LenientProviderOidVisitor;

        impl<'de> Visitor<'de> for LenientProviderOidVisitor {
            type Value = LenientProviderOid;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a string or an ignored malformed value")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(LenientProviderOid::from_borrowed(value))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(LenientProviderOid::from_owned(value))
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
                Ok(LenientProviderOid::default())
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
                Ok(LenientProviderOid::default())
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
                Ok(LenientProviderOid::default())
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
                Ok(LenientProviderOid::default())
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(LenientProviderOid::default())
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(LenientProviderOid::default())
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                while sequence.next_element::<IgnoredAny>()?.is_some() {}
                Ok(LenientProviderOid::default())
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(LenientProviderOid::default())
            }
        }

        deserializer.deserialize_any(LenientProviderOidVisitor)
    }
}

impl<'de> Deserialize<'de> for RawObjectId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawObjectIdVisitor;

        impl<'de> Visitor<'de> for RawObjectIdVisitor {
            type Value = RawObjectId;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an object ID or an ignored malformed value")
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
                Ok(RawObjectId::default())
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
                Ok(RawObjectId::default())
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
                Ok(RawObjectId::default())
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
                Ok(RawObjectId::default())
            }

            fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
                Ok(RawObjectId::default())
            }

            fn visit_string<E>(self, _value: String) -> Result<Self::Value, E> {
                Ok(RawObjectId::default())
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(RawObjectId::default())
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(RawObjectId::default())
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                while sequence.next_element::<IgnoredAny>()?.is_some() {}
                Ok(RawObjectId::default())
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut object_id = RawObjectId::default();
                while let Some(field) = map.next_key::<RawObjectIdField>()? {
                    match field {
                        RawObjectIdField::Oid => {
                            let candidate = map.next_value::<LenientProviderOid>()?;
                            if object_id.oid.is_none() && object_id.invalid_oid_digest.is_none() {
                                object_id.oid = candidate.value;
                                object_id.invalid_oid_digest = candidate.invalid_digest;
                            }
                        }
                        RawObjectIdField::Other => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(object_id)
            }
        }

        deserializer.deserialize_any(RawObjectIdVisitor)
    }
}

#[derive(Default)]
struct RawReward {
    counted_items: Vec<RawCountedItem>,
}

#[derive(Deserialize)]
struct RawCountedItem {
    #[serde(rename = "ItemType", default)]
    item_type: Option<String>,
    #[serde(rename = "ItemCount", default)]
    item_count: Option<u64>,
}

#[derive(Deserialize)]
struct CappedCountedItems(
    #[serde(deserialize_with = "deserialize_counted_items")] Vec<RawCountedItem>,
);

pub(super) const SYNDICATE_MISSION_INPUT_MAX: usize = 128;
const CYCLE_SYNDICATE_TAGS: [&str; 2] = ["CetusSyndicate", "ZarimanSyndicate"];

fn deserialize_cycle_syndicate_missions<'de, D>(
    deserializer: D,
) -> Result<Vec<RawSyndicateMission>, D::Error>
where
    D: Deserializer<'de>,
{
    struct CycleSyndicateVisitor;

    impl<'de> Visitor<'de> for CycleSyndicateVisitor {
        type Value = Vec<RawSyndicateMission>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                formatter,
                "a SyndicateMissions sequence with at most {SYNDICATE_MISSION_INPUT_MAX} entries"
            )
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut retained = Vec::with_capacity(CYCLE_SYNDICATE_TAGS.len());
            for _ in 0..SYNDICATE_MISSION_INPUT_MAX {
                let Some(mission) = sequence.next_element::<RawSyndicateMission>()? else {
                    return Ok(retained);
                };
                let wanted = mission
                    .tag
                    .as_deref()
                    .is_some_and(|tag| CYCLE_SYNDICATE_TAGS.contains(&tag));
                let duplicate = retained
                    .iter()
                    .any(|existing| existing.tag.as_deref() == mission.tag.as_deref());
                if wanted && !duplicate {
                    retained.push(mission);
                }
            }
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(A::Error::custom(format_args!(
                    "too many entries (maximum {SYNDICATE_MISSION_INPUT_MAX})"
                )));
            }
            Ok(retained)
        }
    }

    deserializer.deserialize_seq(CycleSyndicateVisitor)
}

macro_rules! capped_vec_deserializer {
    ($name:ident, $item:ty, $max:expr) => {
        fn $name<'de, D>(deserializer: D) -> Result<Vec<$item>, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserialize_capped_vec::<D, $item, { $max }>(deserializer)
        }
    };
}

capped_vec_deserializer!(deserialize_void_traders, RawVoidTrader, CYCLE_LIST_MAX);
capped_vec_deserializer!(
    deserialize_active_missions,
    RawActiveMission,
    FISSURE_LIST_MAX
);
capped_vec_deserializer!(deserialize_void_storms, RawVoidStorm, FISSURE_LIST_MAX);
capped_vec_deserializer!(deserialize_sorties, RawSortie, ACTIVITY_MISSION_MAX);
capped_vec_deserializer!(
    deserialize_lite_sorties,
    RawLiteSortie,
    ACTIVITY_MISSION_MAX
);
capped_vec_deserializer!(deserialize_invasions, RawInvasion, INVASION_LIST_MAX);
capped_vec_deserializer!(
    deserialize_sortie_variants,
    RawSortieVariant,
    ACTIVITY_MISSION_MAX
);
capped_vec_deserializer!(
    deserialize_archon_missions,
    RawArchonMission,
    ACTIVITY_MISSION_MAX
);
capped_vec_deserializer!(deserialize_counted_items, RawCountedItem, INVASION_LIST_MAX);

impl<'de> Deserialize<'de> for RawReward {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawRewardVisitor;

        impl<'de> Visitor<'de> for RawRewardVisitor {
            type Value = RawReward;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an invasion reward object or an empty sequence")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut counted_items = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "countedItems" {
                        if counted_items.is_some() {
                            return Err(A::Error::duplicate_field("countedItems"));
                        }
                        counted_items = Some(map.next_value::<CappedCountedItems>()?.0);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(RawReward {
                    counted_items: counted_items.unwrap_or_default(),
                })
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                if sequence.next_element::<IgnoredAny>()?.is_some() {
                    return Err(A::Error::custom(
                        "too many entries in legacy reward sequence (maximum 0)",
                    ));
                }
                Ok(RawReward::default())
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(RawReward::default())
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(RawReward::default())
            }
        }

        deserializer.deserialize_any(RawRewardVisitor)
    }
}

/// Cetus day (100m) / night (50m), seconds.
pub(super) const CETUS_DAY_SECS: u64 = 6_000;
pub(super) const CETUS_NIGHT_SECS: u64 = 3_000;
/// Vallis loop (WFCD `VallisCycle`, epoch refreshed 2026-02-04).
pub(super) const VALLIS_EPOCH_MS: i64 = 1_770_234_408_000;
const VALLIS_LOOP_MS: i64 = 1_600_000;
const VALLIS_COLD_MS: i64 = 1_200_000; // loop − warm (400s)
/// Zariman 5h full / 2.5h half (WFCD `ZarimanCycle`).
const ZARIMAN_EPOCH_MS: i64 = 1_655_182_800_000;
const ZARIMAN_FULL_MS: i64 = 18_000_000;
const ZARIMAN_HALF_MS: i64 = 9_000_000;

pub fn parse_worldstate(
    bytes: &[u8],
    fetched_at_secs: u64,
) -> Result<WorldstateSnapshot, ParseError> {
    let raw: RawWorldstate = serde_json::from_slice(bytes)
        .map_err(|error| ParseError(bound_chars(&error.to_string(), ERROR_MAX_CHARS)))?;
    // Keep DE `Time` for diagnostics; cycle math uses wall clock at fetch so
    // state/expiry match warframestat.us (WFCD uses `Date.now()`, not `Time`).
    let server_time_secs = raw.time.unwrap_or(fetched_at_secs);
    let now_ms = i64::try_from(fetched_at_secs.saturating_mul(1_000)).unwrap_or(i64::MAX);

    let cetus_bounty_end = syndicate_expiry(&raw.syndicate_missions, "CetusSyndicate");
    let zariman_bounty_end = syndicate_expiry(&raw.syndicate_missions, "ZarimanSyndicate");

    let mut cycles = Vec::with_capacity(4);
    if let Some(end) = cetus_bounty_end {
        let cetus = cetus_like_cycle("cetus", "Cetus", end, now_ms, "day", "night");
        let cambion = CycleStatus {
            id: "cambion".to_owned(),
            label: "Cambion".to_owned(),
            // Fass aligns with Cetus day; Vome with night (WFCD CambionCycle).
            state: Some(if cetus.state.as_deref() == Some("day") {
                "fass".to_owned()
            } else {
                "vome".to_owned()
            }),
            expires_at_secs: cetus.expires_at_secs,
        };
        cycles.push(cetus);
        cycles.push(cambion);
    }
    cycles.push(vallis_at(now_ms));
    if let Some(end) = zariman_bounty_end {
        cycles.push(zariman_from_bounty(end, now_ms));
    }
    cycles.truncate(CYCLE_LIST_MAX);

    // Daily standing / login reset is midnight UTC — not Sortie (16:00 UTC).
    let daily_reset_at_secs = Some(next_utc_midnight(fetched_at_secs));

    let baro = select_baro(&raw.void_traders, fetched_at_secs);

    let mut fissures = Vec::new();
    for mission in &raw.active_missions {
        if let Some(fissure) = star_chart_fissure(mission) {
            fissures.push(fissure);
        }
    }
    for storm in &raw.void_storms {
        if let Some(fissure) = void_storm_fissure(storm) {
            fissures.push(fissure);
        }
    }
    // Drop already-expired rows using wall clock at fetch.
    fissures.retain(|fissure| fissure.expires_at_secs > fetched_at_secs);
    fissures.sort_by_key(|fissure| (fissure.expires_at_secs, fissure.node.clone()));
    if fissures.len() > FISSURE_LIST_MAX {
        fissures.truncate(FISSURE_LIST_MAX);
    }

    let sortie = raw
        .sorties
        .first()
        .and_then(|entry| parse_sortie(entry, fetched_at_secs));
    let archon = raw
        .lite_sorties
        .first()
        .and_then(|entry| parse_archon(entry, fetched_at_secs));

    let mut invasions: Vec<InvasionMission> =
        raw.invasions.iter().filter_map(parse_invasion).collect();
    if invasions.len() > INVASION_LIST_MAX {
        invasions.truncate(INVASION_LIST_MAX);
    }

    Ok(WorldstateSnapshot {
        server_time_secs,
        fetched_at_secs,
        last_attempt_at_secs: fetched_at_secs,
        cycles,
        daily_reset_at_secs,
        baro,
        fissures,
        sortie,
        archon,
        invasions,
        error: None,
    })
}

pub fn fissure_source(fissure: &FissureMission) -> overcrow_config::FissureSource {
    use overcrow_config::FissureSource;
    if fissure.is_storm {
        FissureSource::Railjack
    } else if fissure.steel_path {
        FissureSource::SteelPath
    } else {
        FissureSource::Normal
    }
}

#[cfg(test)]
pub fn filter_fissures(
    fissures: &[FissureMission],
    prefs: &WarframePrefs,
    now_secs: u64,
) -> Vec<FissureMission> {
    fissures
        .iter()
        .filter(|fissure| fissure.expires_at_secs > now_secs)
        .filter(|fissure| prefs.era_enabled(fissure.era))
        .filter(|fissure| prefs.source_enabled(fissure_source(fissure)))
        .cloned()
        .collect()
}

pub fn invasion_on_watchlist(invasion: &InvasionMission, prefs: &WarframePrefs) -> bool {
    invasion
        .attacker_reward
        .as_ref()
        .is_some_and(|reward| prefs.invasion_watchlisted(&reward.item_key))
        || invasion
            .defender_reward
            .as_ref()
            .is_some_and(|reward| prefs.invasion_watchlisted(&reward.item_key))
}

#[cfg(test)]
pub fn filter_invasions(
    invasions: &[InvasionMission],
    prefs: &WarframePrefs,
) -> Vec<InvasionMission> {
    let mut filtered: Vec<InvasionMission> = invasions
        .iter()
        .filter(|invasion| !prefs.invasion_hide_completed || !invasion.completed)
        .filter(|invasion| invasion_matches_resource_filter(invasion, prefs))
        .cloned()
        .collect();
    filtered.sort_by(|left, right| {
        let left_watch = invasion_on_watchlist(left, prefs);
        let right_watch = invasion_on_watchlist(right, prefs);
        let left_done = prefs.activity_is_done(&super::super::activity_keys::invasion_done_key(
            &left.instance_id,
        ));
        let right_done = prefs.activity_is_done(&super::super::activity_keys::invasion_done_key(
            &right.instance_id,
        ));
        // Watchlist first, then open before user-finished (optional), then node.
        right_watch
            .cmp(&left_watch)
            .then_with(|| {
                if prefs.invasion_push_done_down {
                    left_done.cmp(&right_done)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| left.node.cmp(&right.node))
    });
    filtered
}

#[cfg(test)]
fn invasion_matches_resource_filter(invasion: &InvasionMission, prefs: &WarframePrefs) -> bool {
    let keys: Vec<&str> = [
        invasion
            .attacker_reward
            .as_ref()
            .map(|r| r.item_key.as_str()),
        invasion
            .defender_reward
            .as_ref()
            .map(|r| r.item_key.as_str()),
    ]
    .into_iter()
    .flatten()
    .collect();
    // No-reward invasions only appear when the filter is open (show all).
    if keys.is_empty() {
        return prefs.invasion_resource_filter.is_empty();
    }
    prefs.invasion_matches_resource_filter(&keys)
}

pub fn invasion_reward_catalog(invasions: &[InvasionMission]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for invasion in invasions {
        for reward in [&invasion.attacker_reward, &invasion.defender_reward]
            .into_iter()
            .flatten()
        {
            if !out.iter().any(|(key, _)| key == &reward.item_key) {
                out.push((reward.item_key.clone(), reward.label.clone()));
            }
        }
    }
    out.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    out
}

fn parse_sortie(raw: &RawSortie, now_secs: u64) -> Option<SortieMission> {
    let expires_at_secs = parse_epoch_secs(raw.expiry.as_ref())?;
    if expires_at_secs <= now_secs {
        return None;
    }
    let mut missions: Vec<ActivityMission> = raw
        .variants
        .iter()
        .map(|variant| ActivityMission {
            mission_type: bound_chars(
                variant.mission_type.as_deref().unwrap_or("—"),
                STRING_MAX_CHARS,
            ),
            node: bound_chars(variant.node.as_deref().unwrap_or("—"), STRING_MAX_CHARS),
            modifier: variant.modifier_type.as_deref().map(format_modifier),
        })
        .collect();
    missions.truncate(ACTIVITY_MISSION_MAX);
    Some(SortieMission {
        boss: format_boss(raw.boss.as_deref().unwrap_or("—")),
        expires_at_secs,
        missions,
    })
}

fn parse_archon(raw: &RawLiteSortie, now_secs: u64) -> Option<ArchonHunt> {
    let expires_at_secs = parse_epoch_secs(raw.expiry.as_ref())?;
    if expires_at_secs <= now_secs {
        return None;
    }
    let mut missions: Vec<ActivityMission> = raw
        .missions
        .iter()
        .map(|mission| ActivityMission {
            mission_type: bound_chars(
                mission.mission_type.as_deref().unwrap_or("—"),
                STRING_MAX_CHARS,
            ),
            node: bound_chars(mission.node.as_deref().unwrap_or("—"), STRING_MAX_CHARS),
            modifier: None,
        })
        .collect();
    missions.truncate(ACTIVITY_MISSION_MAX);
    Some(ArchonHunt {
        boss: format_boss(raw.boss.as_deref().unwrap_or("—")),
        expires_at_secs,
        missions,
    })
}

fn parse_invasion(raw: &RawInvasion) -> Option<InvasionMission> {
    let node = raw.node.as_deref()?.trim();
    if node.is_empty() {
        return None;
    }
    let node = bound_chars(node, STRING_MAX_CHARS);
    let attacker_faction = format_faction(raw.faction.as_deref().unwrap_or("—"));
    let defender_faction = format_faction(raw.defender_faction.as_deref().unwrap_or("—"));
    let attacker_reward = parse_reward(&raw.attacker_reward);
    let defender_reward = parse_reward(&raw.defender_reward);
    let goal = raw.goal.unwrap_or(0);
    let provider_id = raw.id.as_ref().and_then(|id| id.oid.as_deref());
    let invalid_provider_id_digest = raw.id.as_ref().and_then(|id| id.invalid_oid_digest);
    let instance_id = provider_id.map(str::to_owned).unwrap_or_else(|| {
        fallback_invasion_instance_id(
            &node,
            &attacker_faction,
            &defender_faction,
            goal,
            attacker_reward
                .as_ref()
                .map(|reward| reward.item_key.as_str()),
            defender_reward
                .as_ref()
                .map(|reward| reward.item_key.as_str()),
            invalid_provider_id_digest,
        )
    });
    Some(InvasionMission {
        instance_id,
        node,
        attacker_faction,
        defender_faction,
        attacker_reward,
        defender_reward,
        count: raw.count.unwrap_or(0),
        goal,
        completed: raw.completed,
    })
}

fn sanitize_invasion_instance_id(raw: &str) -> Option<String> {
    (!raw.is_empty()
        && raw.chars().count() <= invasion_instance_id_max_chars()
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then(|| raw.to_owned())
}

fn fallback_invasion_instance_id(
    node: &str,
    attacker_faction: &str,
    defender_faction: &str,
    goal: i64,
    attacker_reward_key: Option<&str>,
    defender_reward_key: Option<&str>,
    invalid_provider_id_digest: Option<u64>,
) -> String {
    let goal = goal.to_string();
    let invalid_provider_id_digest = invalid_provider_id_digest
        .map(|digest| format!("{digest:016x}"))
        .unwrap_or_default();
    let mut hash = FNV_OFFSET_BASIS;
    for field in [
        &invalid_provider_id_digest,
        node,
        attacker_faction,
        defender_faction,
        &goal,
        attacker_reward_key.unwrap_or(""),
        defender_reward_key.unwrap_or(""),
    ] {
        for byte in field.bytes().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    format!("{hash:016x}")
}

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a_digest(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn parse_reward(value: &RawReward) -> Option<RewardLine> {
    let first = value.counted_items.first()?;
    let item_type = first.item_type.as_deref()?;
    let count = first.item_count.unwrap_or(1).min(u64::from(u32::MAX)) as u32;
    Some(RewardLine {
        item_key: item_key(item_type),
        label: format_item(item_type),
        count: count.max(1),
    })
}

fn syndicate_expiry(missions: &[RawSyndicateMission], tag: &str) -> Option<u64> {
    missions
        .iter()
        .find(|mission| mission.tag.as_deref() == Some(tag))
        .and_then(|mission| parse_epoch_secs(mission.expiry.as_ref()))
}

/// WFCD `CetusCycle`: bounty expiry marks end of night; last 50 minutes are night.
fn cetus_like_cycle(
    id: &str,
    label: &str,
    bounty_end_secs: u64,
    now_ms: i64,
    day_label: &str,
    night_label: &str,
) -> CycleStatus {
    // WFCD: `bountiesEndDate.setSeconds(0)` then round phase end to the nearest minute.
    let bounty_end_ms = i64::try_from(bounty_end_secs.saturating_mul(1_000)).unwrap_or(i64::MAX);
    let bounty_end_ms = bounty_end_ms - bounty_end_ms.rem_euclid(60_000);
    let millis_left = bounty_end_ms - now_ms;
    // WFCD: `Number((millisLeft / 1000).toFixed(0))` — round half away from zero-ish.
    let seconds_to_night_end = div_round(millis_left, 1_000);
    let is_day = seconds_to_night_end > CETUS_NIGHT_SECS as i64;
    let seconds_remaining = if is_day {
        seconds_to_night_end - CETUS_NIGHT_SECS as i64
    } else {
        seconds_to_night_end
    };
    let expires_at_secs = if seconds_remaining > 0 {
        // WFCD: `Math.round((now + millisLeft) / 60000) * 60000`
        let expiry_ms = div_round(now_ms + seconds_remaining * 1_000, 60_000) * 60_000;
        u64::try_from(expiry_ms / 1_000).unwrap_or(bounty_end_secs)
    } else {
        u64::try_from(bounty_end_ms / 1_000).unwrap_or(bounty_end_secs)
    };
    CycleStatus {
        id: id.to_owned(),
        label: label.to_owned(),
        state: Some(if is_day {
            day_label.to_owned()
        } else {
            night_label.to_owned()
        }),
        expires_at_secs,
    }
}

/// Vallis warm/cold at wall-clock `now_ms` (pure epoch loop).
pub(super) fn vallis_at(now_ms: i64) -> CycleStatus {
    let since_last = (now_ms - VALLIS_EPOCH_MS).rem_euclid(VALLIS_LOOP_MS);
    let to_next_full = VALLIS_LOOP_MS - since_last;
    let is_warm = to_next_full > VALLIS_COLD_MS;
    let to_next_minor = if to_next_full < VALLIS_COLD_MS {
        to_next_full
    } else {
        to_next_full - VALLIS_COLD_MS
    };
    // WFCD keeps second precision on Vallis expiry (no minute rounding).
    let expires_at_secs = u64::try_from((now_ms + to_next_minor) / 1_000).unwrap_or(0);
    CycleStatus {
        id: "vallis".to_owned(),
        label: "Vallis".to_owned(),
        state: Some(if is_warm {
            "warm".to_owned()
        } else {
            "cold".to_owned()
        }),
        expires_at_secs,
    }
}

/// Zariman corpus/grineer from wall clock (next faction flip). Fallback when the
/// bounty window has already lapsed.
pub(super) fn zariman_at(now_ms: i64) -> CycleStatus {
    let cycle_time_elapsed = (now_ms - ZARIMAN_EPOCH_MS).rem_euclid(ZARIMAN_FULL_MS);
    let cycle_time_left = ZARIMAN_FULL_MS - cycle_time_elapsed;
    let is_corpus = cycle_time_left > ZARIMAN_HALF_MS;
    let to_flip = if is_corpus {
        cycle_time_left - ZARIMAN_HALF_MS
    } else {
        cycle_time_left
    };
    let expires_at_secs = u64::try_from((now_ms + to_flip.max(1_000)) / 1_000).unwrap_or(0);
    CycleStatus {
        id: "zariman".to_owned(),
        label: "Zariman".to_owned(),
        state: Some(if is_corpus {
            "corpus".to_owned()
        } else {
            "grineer".to_owned()
        }),
        expires_at_secs,
    }
}

fn zariman_from_bounty(bounty_end_secs: u64, now_ms: i64) -> CycleStatus {
    // Remaining time tracks the Zariman bounty window (matches warframestat).
    let bounty_end_ms = i64::try_from(bounty_end_secs.saturating_mul(1_000)).unwrap_or(i64::MAX);
    let bounty_end_ms = bounty_end_ms - bounty_end_ms.rem_euclid(60_000);
    let millis_left = bounty_end_ms - now_ms;
    if millis_left <= 0 {
        // Bounty already lapsed — fall back to pure epoch phase so UI stays live.
        return zariman_at(now_ms);
    }
    let expires_at_secs = u64::try_from((now_ms + millis_left) / 1_000).unwrap_or(bounty_end_secs);

    // Faction: WFCD modulus against a known Corpus epoch (half-cycle 2.5h of 5h).
    let bounties_clone = bounty_end_ms - 5_000;
    let cycle_time_elapsed = (bounties_clone - ZARIMAN_EPOCH_MS).rem_euclid(ZARIMAN_FULL_MS);
    let cycle_time_left = ZARIMAN_FULL_MS - cycle_time_elapsed;
    let is_corpus = cycle_time_left > ZARIMAN_HALF_MS;

    CycleStatus {
        id: "zariman".to_owned(),
        label: "Zariman".to_owned(),
        state: Some(if is_corpus {
            "corpus".to_owned()
        } else {
            "grineer".to_owned()
        }),
        expires_at_secs,
    }
}

/// Integer division rounded to nearest (half away from zero), like JS `Math.round` for positives.
fn div_round(numerator: i64, denominator: i64) -> i64 {
    if denominator == 0 {
        return 0;
    }
    if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    }
}

fn star_chart_fissure(mission: &RawActiveMission) -> Option<FissureMission> {
    let modifier = mission.modifier.as_deref()?;
    let era = FissureEra::from_void_modifier(modifier)?;
    let expires_at_secs = parse_epoch_secs(mission.expiry.as_ref())?;
    Some(FissureMission {
        era,
        mission_type: bound_chars(
            mission.mission_type.as_deref().unwrap_or("—"),
            STRING_MAX_CHARS,
        ),
        node: bound_chars(mission.node.as_deref().unwrap_or("—"), STRING_MAX_CHARS),
        expires_at_secs,
        steel_path: mission.hard,
        is_storm: false,
    })
}

fn void_storm_fissure(storm: &RawVoidStorm) -> Option<FissureMission> {
    let tier = storm.active_mission_tiers.as_deref()?;
    let era = FissureEra::from_void_modifier(tier)?;
    let expires_at_secs = parse_epoch_secs(storm.expiry.as_ref())?;
    Some(FissureMission {
        era,
        mission_type: "Void Storm".to_owned(),
        node: bound_chars(storm.node.as_deref().unwrap_or("—"), STRING_MAX_CHARS),
        expires_at_secs,
        steel_path: false,
        is_storm: true,
    })
}

fn select_baro(traders: &[RawVoidTrader], now_secs: u64) -> Option<BaroStatus> {
    let mut candidates: Vec<_> = traders
        .iter()
        .filter(|trader| {
            trader
                .character
                .as_deref()
                .is_some_and(|name| name.to_ascii_lowercase().contains("baro"))
        })
        .filter_map(|trader| {
            let activation_secs = parse_epoch_secs(trader.activation.as_ref())?;
            let expiry_secs = parse_epoch_secs(trader.expiry.as_ref())?;
            (expiry_secs > now_secs).then_some((trader, activation_secs, expiry_secs))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|(trader, activation, expiry)| {
        let present = *activation <= now_secs && now_secs < *expiry;
        let known = trader
            .node
            .as_deref()
            .is_some_and(|node| resolve_node(node).planet.is_some());
        (
            if present { 0u8 } else { 1u8 },
            *activation,
            if known { 0u8 } else { 1u8 },
            trader.node.clone().unwrap_or_default(),
        )
    });
    let (trader, activation_secs, expiry_secs) = candidates.into_iter().next()?;
    Some(BaroStatus {
        present: activation_secs <= now_secs && now_secs < expiry_secs,
        activation_secs,
        expiry_secs,
        location: trader.node.as_deref().map(format_node),
    })
}

fn next_utc_midnight(server_time_secs: u64) -> u64 {
    let day = server_time_secs / 86_400;
    (day + 1) * 86_400
}

fn parse_epoch_secs(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if let Some(number) = value.as_u64() {
        return Some(normalize_epoch(number));
    }
    if let Some(number) = value.as_i64() {
        return u64::try_from(number).ok().map(normalize_epoch);
    }
    if let Some(text) = value.as_str() {
        return text.parse::<u64>().ok().map(normalize_epoch);
    }
    let date = value.get("$date")?;
    if let Some(number) = date.as_u64() {
        return Some(normalize_epoch(number));
    }
    if let Some(number) = date.as_i64() {
        return u64::try_from(number).ok().map(normalize_epoch);
    }
    if let Some(text) = date.as_str() {
        return text.parse::<u64>().ok().map(normalize_epoch);
    }
    let number_long = date.get("$numberLong")?.as_str()?;
    number_long.parse::<u64>().ok().map(normalize_epoch)
}

fn normalize_epoch(value: u64) -> u64 {
    if value > 10_000_000_000 {
        value / 1_000
    } else {
        value
    }
}

#[cfg(test)]
mod cycle_tests {
    use super::{cetus_like_cycle, div_round, next_utc_midnight, vallis_at};

    #[test]
    fn cetus_last_fifty_minutes_are_night() {
        // Bounty ends at t=10_000s (floored to minute = 9960). At 8_000s → night.
        let cycle = cetus_like_cycle("cetus", "Cetus", 10_000, 8_000_000, "day", "night");
        assert_eq!(cycle.state.as_deref(), Some("night"));
        // Expiry is rounded to the nearest minute (WFCD).
        assert_eq!(cycle.expires_at_secs, 9_960);
    }

    #[test]
    fn cetus_before_last_fifty_minutes_is_day() {
        // ~3960s to floored bounty end → day; remaining until night = 960s.
        let cycle = cetus_like_cycle("cetus", "Cetus", 10_000, 6_000_000, "day", "night");
        assert_eq!(cycle.state.as_deref(), Some("day"));
        assert_eq!(cycle.expires_at_secs, 6_960);
    }

    #[test]
    fn cetus_matches_warframestat_day_expiry_sample() {
        // Live sample (2026-07-17): Ostron expiry 00:29:52Z → day ends 23:39:00Z.
        let bounty_end = 1_784_334_592; // 2026-07-18T00:29:52Z
        let now_ms = 1_784_327_796_000; // ~2026-07-17T22:36:36Z wall
        let cycle = cetus_like_cycle("cetus", "Cetus", bounty_end, now_ms, "day", "night");
        assert_eq!(cycle.state.as_deref(), Some("day"));
        assert_eq!(cycle.expires_at_secs, 1_784_331_540); // 2026-07-17T23:39:00Z
    }

    #[test]
    fn daily_reset_is_next_utc_midnight() {
        // 2026-07-17 19:30 UTC-ish
        let t = 1_784_316_600;
        let reset = next_utc_midnight(t);
        assert_eq!(reset % 86_400, 0);
        assert!(reset > t);
        assert!(reset - t <= 86_400);
    }

    #[test]
    fn vallis_matches_warframestat_sample() {
        // Sample matching api.warframestat.us/pc/vallisCycle around 22:36 UTC.
        let now_ms = 1_784_327_796_000_i64;
        let cycle = vallis_at(now_ms);
        assert_eq!(cycle.state.as_deref(), Some("cold"));
        assert_eq!(cycle.expires_at_secs, 1_784_328_808); // 2026-07-17T22:53:28Z
    }

    #[test]
    fn div_round_matches_js_math_round_for_positives() {
        assert_eq!(div_round(2_500, 1_000), 3);
        assert_eq!(div_round(2_499, 1_000), 2);
        assert_eq!(div_round(60_000, 60_000), 1);
    }
}
