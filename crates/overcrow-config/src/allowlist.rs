use std::{collections::BTreeSet, path::PathBuf};

use crate::LifecycleSettings;

/// Process identity reported by the platform-specific process classifier.
///
/// Candidate metadata is informational. Authorization always requires an exact
/// selected Steam application ID or executable path.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessIdentity {
    pub steam_app_id: Option<u32>,
    pub executable_chain: Vec<PathBuf>,
    pub game_candidate: bool,
}

/// Exact identities selected by the user in validated lifecycle settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GameAllowlist {
    steam_app_ids: BTreeSet<u32>,
    manual_executables: BTreeSet<PathBuf>,
}

impl GameAllowlist {
    pub fn from_settings(settings: &LifecycleSettings) -> Self {
        if !settings.enabled || settings.clone().validate().is_err() {
            return Self::default();
        }

        Self {
            steam_app_ids: settings.selected_steam_app_ids.clone(),
            manual_executables: settings
                .manual_games
                .iter()
                .map(|game| game.executable.clone())
                .collect(),
        }
    }

    pub fn allows_identity(&self, identity: &ProcessIdentity) -> bool {
        identity
            .steam_app_id
            .filter(|id| *id != 0)
            .is_some_and(|id| self.steam_app_ids.contains(&id))
            || identity
                .executable_chain
                .iter()
                .any(|path| self.manual_executables.contains(path))
    }

    /// Classifies each unique PID in ascending order without retaining PID state.
    pub fn any_selected_process(
        &self,
        pids: impl IntoIterator<Item = u32>,
        mut classify: impl FnMut(u32) -> ProcessIdentity,
    ) -> bool {
        pids.into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .any(|pid| self.allows_identity(&classify(pid)))
    }
}
