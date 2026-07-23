use std::{cmp::Ordering, sync::Arc};

use overcrow_config::WarframePrefs;

use super::{
    WorldstateSnapshot, fissure_source, invasion_done_key, invasion_on_watchlist,
    invasion_reward_catalog,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvasionCompactLabel {
    pub attacker: Option<String>,
    pub defender: Option<String>,
    pub node: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DerivedRecomputationCounts {
    pub reward_catalog: usize,
    pub fissure_indices: usize,
    pub invasion_indices: usize,
    pub invasion_labels: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LabelKey {
    worldstate_revision: u64,
    prefs_revision: u64,
    width_bucket: u32,
}

#[derive(Default)]
struct InvasionLabelCache {
    key: Option<LabelKey>,
    values: Arc<[InvasionCompactLabel]>,
}

pub struct WarframeDerivedCache {
    worldstate_revision: Option<u64>,
    prefs_revision: u64,
    reward_catalog: Arc<[(String, String)]>,
    fissure_indices: Arc<[usize]>,
    invasion_indices: Arc<[usize]>,
    invasion_labels: InvasionLabelCache,
    filter_key: Option<(u64, u64)>,
    recomputation_counts: DerivedRecomputationCounts,
}

impl Default for WarframeDerivedCache {
    fn default() -> Self {
        Self {
            worldstate_revision: None,
            prefs_revision: 0,
            reward_catalog: Arc::from([]),
            fissure_indices: Arc::from([]),
            invasion_indices: Arc::from([]),
            invasion_labels: InvasionLabelCache::default(),
            filter_key: None,
            recomputation_counts: DerivedRecomputationCounts::default(),
        }
    }
}

impl WarframeDerivedCache {
    pub fn sync(
        &mut self,
        snapshot: &WorldstateSnapshot,
        worldstate_revision: u64,
        prefs: &WarframePrefs,
        prefs_revision: u64,
    ) {
        let provider_changed = self.worldstate_revision != Some(worldstate_revision);
        let prefs_changed = self.filter_key != Some((worldstate_revision, prefs_revision));

        if provider_changed {
            self.reward_catalog = invasion_reward_catalog(&snapshot.invasions).into();
            self.recomputation_counts.reward_catalog += 1;
        }

        if provider_changed || prefs_changed {
            self.fissure_indices = snapshot
                .fissures
                .iter()
                .enumerate()
                .filter(|(_, fissure)| prefs.era_enabled(fissure.era))
                .filter(|(_, fissure)| prefs.source_enabled(fissure_source(fissure)))
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
                .into();
            self.recomputation_counts.fissure_indices += 1;

            let mut invasion_indices = snapshot
                .invasions
                .iter()
                .enumerate()
                .filter(|(_, invasion)| !prefs.invasion_hide_completed || !invasion.completed)
                .filter(|(_, invasion)| {
                    let mut keys = [""; 2];
                    let mut len = 0;
                    if let Some(reward) = &invasion.attacker_reward {
                        keys[len] = &reward.item_key;
                        len += 1;
                    }
                    if let Some(reward) = &invasion.defender_reward {
                        keys[len] = &reward.item_key;
                        len += 1;
                    }
                    if len == 0 {
                        prefs.invasion_resource_filter.is_empty()
                    } else {
                        prefs.invasion_matches_resource_filter(&keys[..len])
                    }
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            invasion_indices.sort_by(|left, right| {
                let left = &snapshot.invasions[*left];
                let right = &snapshot.invasions[*right];
                let left_watch = invasion_on_watchlist(left, prefs);
                let right_watch = invasion_on_watchlist(right, prefs);
                let left_done = prefs.activity_is_done(&invasion_done_key(&left.instance_id));
                let right_done = prefs.activity_is_done(&invasion_done_key(&right.instance_id));
                right_watch
                    .cmp(&left_watch)
                    .then_with(|| {
                        if prefs.invasion_push_done_down {
                            left_done.cmp(&right_done)
                        } else {
                            Ordering::Equal
                        }
                    })
                    .then_with(|| left.node.cmp(&right.node))
            });
            self.invasion_indices = invasion_indices.into();
            self.recomputation_counts.invasion_indices += 1;
        }

        self.worldstate_revision = Some(worldstate_revision);
        self.prefs_revision = prefs_revision;
        self.filter_key = Some((worldstate_revision, prefs_revision));
    }

    pub fn reward_catalog(&self) -> &Arc<[(String, String)]> {
        &self.reward_catalog
    }

    pub fn fissure_indices(&self) -> &Arc<[usize]> {
        &self.fissure_indices
    }

    pub fn invasion_indices(&self) -> &Arc<[usize]> {
        &self.invasion_indices
    }

    pub fn compact_invasion_labels(
        &mut self,
        worldstate_revision: u64,
        prefs_revision: u64,
        available_width: f32,
        build: impl FnOnce(f32) -> Vec<InvasionCompactLabel>,
    ) -> Arc<[InvasionCompactLabel]> {
        let width_bucket = width_bucket(available_width);
        let key = LabelKey {
            worldstate_revision,
            prefs_revision,
            width_bucket,
        };
        if self.invasion_labels.key != Some(key) {
            self.invasion_labels.values = build(width_bucket as f32 * 8.0).into();
            self.invasion_labels.key = Some(key);
            self.recomputation_counts.invasion_labels += 1;
        }
        Arc::clone(&self.invasion_labels.values)
    }

    pub fn recomputation_counts(&self) -> DerivedRecomputationCounts {
        self.recomputation_counts
    }
}

fn width_bucket(width: f32) -> u32 {
    if width.is_finite() && width > 0.0 {
        (width / 8.0).floor().min(u32::MAX as f32) as u32
    } else {
        0
    }
}
