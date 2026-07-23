//! Stable local keys for checklist state in `WarframePrefs::activity_done`.

use overcrow_config::WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS;

use super::model::WorldstateSnapshot;

const INVASION_DONE_KEY_PREFIX: &str = "invasion:";

pub(super) fn invasion_instance_id_max_chars() -> usize {
    WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS - INVASION_DONE_KEY_PREFIX.chars().count()
}

pub fn sortie_mission_key(expires_at_secs: u64, index: usize) -> String {
    format!("sortie:{expires_at_secs}:{index}")
}

pub fn archon_mission_key(expires_at_secs: u64, index: usize) -> String {
    format!("archon:{expires_at_secs}:{index}")
}

pub fn invasion_done_key(instance_id: &str) -> String {
    format!("{INVASION_DONE_KEY_PREFIX}{instance_id}")
}

pub fn sortie_mission_keys(expires_at_secs: u64, count: usize) -> Vec<String> {
    (0..count)
        .map(|index| sortie_mission_key(expires_at_secs, index))
        .collect()
}

pub fn archon_mission_keys(expires_at_secs: u64, count: usize) -> Vec<String> {
    (0..count)
        .map(|index| archon_mission_key(expires_at_secs, index))
        .collect()
}

pub fn current_activity_done_keys(snapshot: &WorldstateSnapshot) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(sortie) = &snapshot.sortie {
        keys.extend(sortie_mission_keys(
            sortie.expires_at_secs,
            sortie.missions.len(),
        ));
    }
    if let Some(archon) = &snapshot.archon {
        keys.extend(archon_mission_keys(
            archon.expires_at_secs,
            archon.missions.len(),
        ));
    }
    keys.extend(
        snapshot
            .invasions
            .iter()
            .map(|invasion| invasion_done_key(&invasion.instance_id)),
    );
    keys
}

#[cfg(test)]
mod tests {
    use overcrow_config::WarframePrefs;

    use super::current_activity_done_keys;
    use crate::warframe::model::{
        ActivityMission, ArchonHunt, InvasionMission, SortieMission, WorldstateSnapshot,
    };

    fn mission() -> ActivityMission {
        ActivityMission {
            mission_type: "Extermination".to_owned(),
            node: "SolNode1".to_owned(),
            modifier: None,
        }
    }

    #[test]
    fn snapshot_keys_prune_a_full_expired_set_before_current_insertion() {
        let snapshot = WorldstateSnapshot {
            sortie: Some(SortieMission {
                boss: "Boss".to_owned(),
                expires_at_secs: 2_000,
                missions: vec![mission()],
            }),
            archon: Some(ArchonHunt {
                boss: "Archon".to_owned(),
                expires_at_secs: 3_000,
                missions: vec![mission()],
            }),
            invasions: vec![InvasionMission {
                instance_id: "provider-object-a".to_owned(),
                node: "SolNode1".to_owned(),
                attacker_faction: "Grineer".to_owned(),
                defender_faction: "Corpus".to_owned(),
                attacker_reward: None,
                defender_reward: None,
                count: 0,
                goal: 1,
                completed: false,
            }],
            ..WorldstateSnapshot::default()
        };
        let current = current_activity_done_keys(&snapshot);
        assert_eq!(
            current,
            vec![
                "sortie:2000:0".to_owned(),
                "archon:3000:0".to_owned(),
                "invasion:provider-object-a".to_owned(),
            ]
        );
        let mut prefs = WarframePrefs {
            activity_done: (0..128).map(|index| format!("sortie:{index}:0")).collect(),
            ..WarframePrefs::default()
        };

        prefs.prune_activity_done(&current);
        prefs.toggle_activity_done(&current[2]);

        assert_eq!(prefs.activity_done, vec![current[2].clone()]);
    }
}
