use overcrow_config::{FissureEra, WarframePrefs};

use super::{
    FissureMission, InvasionCompactLabel, InvasionMission, RewardLine, WarframeDerivedCache,
    WorldstateSnapshot, filter_fissures, filter_invasions,
};

fn snapshot() -> WorldstateSnapshot {
    WorldstateSnapshot {
        fissures: vec![
            FissureMission {
                era: FissureEra::Lith,
                mission_type: "Capture".to_owned(),
                node: "SolNode1".to_owned(),
                expires_at_secs: 100,
                steel_path: false,
                is_storm: false,
            },
            FissureMission {
                era: FissureEra::Meso,
                mission_type: "Defense".to_owned(),
                node: "SolNode2".to_owned(),
                expires_at_secs: 200,
                steel_path: true,
                is_storm: false,
            },
        ],
        invasions: vec![
            invasion("plain", "SolNode2", "resource-a"),
            invasion("watched", "SolNode1", "resource-b"),
        ],
        ..WorldstateSnapshot::default()
    }
}

fn invasion(instance_id: &str, node: &str, item_key: &str) -> InvasionMission {
    InvasionMission {
        instance_id: instance_id.to_owned(),
        node: node.to_owned(),
        attacker_faction: "Grineer".to_owned(),
        defender_faction: "Corpus".to_owned(),
        attacker_reward: Some(RewardLine {
            item_key: item_key.to_owned(),
            label: item_key.to_owned(),
            count: 1,
        }),
        defender_reward: None,
        count: 0,
        goal: 1,
        completed: false,
    }
}

fn labels(label: &str) -> Vec<InvasionCompactLabel> {
    vec![InvasionCompactLabel {
        attacker: Some(label.to_owned()),
        defender: None,
        node: label.to_owned(),
    }]
}

#[test]
fn one_hundred_identical_frames_compute_provider_derivations_once() {
    let snapshot = snapshot();
    let prefs = WarframePrefs::default();
    let mut cache = WarframeDerivedCache::default();

    for _ in 0..100 {
        cache.sync(&snapshot, 4, &prefs, 9);
        cache.compact_invasion_labels(4, 9, 100.0, |_| labels("cached"));
    }

    let counts = cache.recomputation_counts();
    assert_eq!(counts.reward_catalog, 1);
    assert_eq!(counts.fissure_indices, 1);
    assert_eq!(counts.invasion_indices, 1);
    assert_eq!(counts.invasion_labels, 1);
}

#[test]
fn provider_and_committed_preference_revisions_invalidate_only_their_dependents() {
    let snapshot = snapshot();
    let mut prefs = WarframePrefs::default();
    let mut cache = WarframeDerivedCache::default();

    cache.sync(&snapshot, 1, &prefs, 0);
    cache.compact_invasion_labels(1, 0, 100.0, |_| labels("initial"));

    cache.sync(&snapshot, 2, &prefs, 0);
    cache.compact_invasion_labels(2, 0, 100.0, |_| labels("provider"));
    let provider = cache.recomputation_counts();
    assert_eq!(provider.reward_catalog, 2);
    assert_eq!(provider.fissure_indices, 2);
    assert_eq!(provider.invasion_indices, 2);
    assert_eq!(provider.invasion_labels, 2);

    prefs.invasion_hide_completed = false;
    cache.sync(&snapshot, 2, &prefs, 1);
    cache.compact_invasion_labels(2, 1, 100.0, |_| labels("prefs"));
    let committed = cache.recomputation_counts();
    assert_eq!(committed.reward_catalog, 2);
    assert_eq!(committed.fissure_indices, 3);
    assert_eq!(committed.invasion_indices, 3);
    assert_eq!(committed.invasion_labels, 3);

    prefs.invasion_hide_completed = true;
    cache.sync(&snapshot, 2, &prefs, 1);
    cache.compact_invasion_labels(2, 1, 100.0, |_| labels("rollback"));
    assert_eq!(cache.recomputation_counts(), committed);
}

#[test]
fn compact_labels_recompute_only_when_crossing_an_eight_point_width_bucket() {
    let snapshot = snapshot();
    let prefs = WarframePrefs::default();
    let mut cache = WarframeDerivedCache::default();
    cache.sync(&snapshot, 3, &prefs, 5);

    let first = cache.compact_invasion_labels(3, 5, 100.0, |width| {
        assert_eq!(width, 96.0);
        labels("first")
    });
    let same_bucket = cache.compact_invasion_labels(3, 5, 103.9, |_| labels("unexpected"));
    assert!(std::sync::Arc::ptr_eq(&first, &same_bucket));

    let next_bucket = cache.compact_invasion_labels(3, 5, 104.0, |width| {
        assert_eq!(width, 104.0);
        labels("next")
    });
    assert!(!std::sync::Arc::ptr_eq(&first, &next_bucket));
    assert_eq!(cache.recomputation_counts().invasion_labels, 2);
    assert_eq!(cache.recomputation_counts().reward_catalog, 1);
    assert_eq!(cache.recomputation_counts().fissure_indices, 1);
    assert_eq!(cache.recomputation_counts().invasion_indices, 1);
}

#[test]
fn cached_indices_preserve_existing_filter_order() {
    let snapshot = snapshot();
    let prefs = WarframePrefs {
        invasion_reward_watchlist: vec!["resource-b".to_owned()],
        ..WarframePrefs::default()
    };
    let mut cache = WarframeDerivedCache::default();
    cache.sync(&snapshot, 1, &prefs, 1);

    let cached_fissures = cache
        .fissure_indices()
        .iter()
        .map(|index| snapshot.fissures[*index].clone())
        .collect::<Vec<_>>();
    let cached_invasions = cache
        .invasion_indices()
        .iter()
        .map(|index| snapshot.invasions[*index].clone())
        .collect::<Vec<_>>();

    assert_eq!(
        cached_fissures,
        filter_fissures(&snapshot.fissures, &prefs, 0)
    );
    assert_eq!(
        cached_invasions,
        filter_invasions(&snapshot.invasions, &prefs)
    );
    assert_eq!(cache.reward_catalog().len(), 2);
}
