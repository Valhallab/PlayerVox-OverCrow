use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

pub use overcrow_config::ProcessIdentity;

const MAX_ANCESTORS: usize = 64;
const STEAM_ID_KEYS: [&str; 2] = ["SteamAppId", "SteamGameId"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessTiming {
    elapsed_at_observation: Duration,
    observed_at: Instant,
}

impl ProcessTiming {
    pub fn new(elapsed_at_observation: Duration, observed_at: Instant) -> Self {
        Self {
            elapsed_at_observation,
            observed_at,
        }
    }

    pub fn elapsed_at(self, now: Instant) -> Duration {
        self.elapsed_at_observation
            .saturating_add(now.saturating_duration_since(self.observed_at))
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: u32,
    pub start_ticks: u64,
    pub timing: Option<ProcessTiming>,
    pub resources: ProcessResources,
    pub name: String,
    pub environment: HashMap<String, String>,
    pub command_line: Vec<String>,
    pub executable: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessResources {
    pub total_cpu_ticks: u64,
    pub resident_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessClassification {
    pub steam_app_id: Option<u32>,
    pub is_game_candidate: bool,
}

pub fn classify_active_pid(
    pid: u32,
    processes: &HashMap<u32, ProcessInfo>,
) -> ProcessClassification {
    let identity = classify_process_identity(pid, processes);
    ProcessClassification {
        steam_app_id: identity.steam_app_id,
        is_game_candidate: identity.game_candidate,
    }
}

pub fn classify_process_identity(
    pid: u32,
    processes: &HashMap<u32, ProcessInfo>,
) -> ProcessIdentity {
    let mut identity = ProcessIdentity::default();
    let mut next_pid = pid;
    let mut seen_pids = HashSet::new();
    let mut seen_executables = HashSet::new();

    for _ in 0..MAX_ANCESTORS {
        if !seen_pids.insert(next_pid) {
            break;
        }

        let Some(process) = processes.get(&next_pid) else {
            break;
        };

        if identity.steam_app_id.is_none() {
            identity.steam_app_id = steam_app_id(process);
        }
        if let Some(executable) = &process.executable
            && seen_executables.insert(executable.clone())
        {
            identity.executable_chain.push(executable.clone());
        }
        identity.game_candidate |= looks_game_related(process);
        next_pid = process.parent_pid;
    }

    identity.game_candidate |= identity.steam_app_id.is_some();
    identity
}

fn steam_app_id(process: &ProcessInfo) -> Option<u32> {
    STEAM_ID_KEYS.iter().find_map(|key| {
        process
            .environment
            .get(*key)?
            .parse::<u32>()
            .ok()
            .filter(|id| *id != 0)
    })
}

fn looks_game_related(process: &ProcessInfo) -> bool {
    related_text(&process.name)
        || process.command_line.iter().any(|part| related_text(part))
        || process
            .executable
            .as_ref()
            .is_some_and(|path| related_text(&path.to_string_lossy()))
}

fn related_text(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("wine") || value.contains("proton") || value.ends_with(".exe")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{
        MAX_ANCESTORS, ProcessClassification, ProcessInfo, classify_active_pid,
        classify_process_identity,
    };

    fn process<const N: usize>(
        pid: u32,
        parent_pid: u32,
        name: &str,
        environment: [(&str, &str); N],
    ) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid,
            start_ticks: 0,
            timing: None,
            resources: Default::default(),
            name: name.to_owned(),
            environment: environment
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
            command_line: Vec::new(),
            executable: Some(PathBuf::from(name)),
        }
    }

    fn map<const N: usize>(processes: [ProcessInfo; N]) -> HashMap<u32, ProcessInfo> {
        processes
            .into_iter()
            .map(|process| (process.pid, process))
            .collect()
    }

    #[test]
    fn reads_direct_steam_app_id() {
        let processes = map([process(20, 10, "cs2", [("SteamAppId", "730")])]);

        let result = classify_active_pid(20, &processes);

        assert_eq!(
            result,
            ProcessClassification {
                steam_app_id: Some(730),
                is_game_candidate: true,
            }
        );
    }

    #[test]
    fn inherits_steam_game_id_from_parent() {
        let processes = map([
            process(10, 1, "steam-launch-wrapper", [("SteamGameId", "570")]),
            process(20, 10, "dota2", []),
        ]);

        let result = classify_active_pid(20, &processes);

        assert_eq!(result.steam_app_id, Some(570));
        assert!(result.is_game_candidate);
    }

    #[test]
    fn inherits_steam_id_from_proton_parent() {
        let processes = map([
            process(10, 1, "proton", [("SteamAppId", "730")]),
            process(20, 10, "cs2.exe", []),
        ]);

        let result = classify_active_pid(20, &processes);

        assert_eq!(result.steam_app_id, Some(730));
        assert!(result.is_game_candidate);
    }

    #[test]
    fn marks_proton_ancestor_as_candidate_without_a_steam_id() {
        let processes = map([process(10, 1, "proton", []), process(20, 10, "game", [])]);

        let result = classify_active_pid(20, &processes);

        assert_eq!(result.steam_app_id, None);
        assert!(result.is_game_candidate);
    }

    #[test]
    fn rejects_unrelated_process() {
        let processes = map([process(20, 1, "firefox", [])]);

        let result = classify_active_pid(20, &processes);

        assert_eq!(
            result,
            ProcessClassification {
                steam_app_id: None,
                is_game_candidate: false,
            }
        );
    }

    #[test]
    fn uses_nearest_valid_id_and_prefers_steam_app_id_on_one_process() {
        let processes = map([
            process(10, 1, "parent", [("SteamAppId", "570")]),
            process(
                20,
                10,
                "child",
                [("SteamGameId", "730"), ("SteamAppId", "440")],
            ),
        ]);

        let result = classify_active_pid(20, &processes);

        assert_eq!(result.steam_app_id, Some(440));
    }

    #[test]
    fn falls_back_to_valid_steam_game_id_on_the_same_process() {
        let processes = map([process(
            20,
            10,
            "child",
            [("SteamAppId", "not-a-number"), ("SteamGameId", "730")],
        )]);

        let result = classify_active_pid(20, &processes);

        assert_eq!(result.steam_app_id, Some(730));
        assert!(result.is_game_candidate);
    }

    #[test]
    fn skips_invalid_steam_ids_and_continues_to_the_parent() {
        let processes = map([
            process(10, 1, "parent", [("SteamGameId", "570")]),
            process(
                20,
                10,
                "child",
                [("SteamAppId", "0"), ("SteamGameId", "not-a-number")],
            ),
        ]);

        let result = classify_active_pid(20, &processes);

        assert_eq!(result.steam_app_id, Some(570));
    }

    #[test]
    fn skips_overflowing_steam_ids_and_continues_to_the_parent() {
        let processes = map([
            process(10, 1, "parent", [("SteamAppId", "570")]),
            process(
                20,
                10,
                "child",
                [
                    ("SteamAppId", "4294967296"),
                    ("SteamGameId", "18446744073709551615"),
                ],
            ),
        ]);

        let result = classify_active_pid(20, &processes);

        assert_eq!(result.steam_app_id, Some(570));
    }

    #[test]
    fn detects_wine_case_insensitively_in_the_command_line() {
        let mut active = process(20, 1, "game", []);
        active.command_line = vec!["/usr/bin/WINE64".to_owned(), "game".to_owned()];
        let processes = map([active]);

        let result = classify_active_pid(20, &processes);

        assert!(result.is_game_candidate);
    }

    #[test]
    fn detects_a_windows_executable_path() {
        let mut active = process(20, 1, "game", []);
        active.executable = Some(PathBuf::from("/games/example/GAME.EXE"));
        let processes = map([active]);

        let result = classify_active_pid(20, &processes);

        assert!(result.is_game_candidate);
    }

    #[test]
    fn stops_after_sixty_four_processes() {
        let mut processes = HashMap::new();
        for pid in 1..=65 {
            let parent_pid = if pid == 65 { 0 } else { pid + 1 };
            let environment = if pid == 65 {
                HashMap::from([("SteamAppId".to_owned(), "730".to_owned())])
            } else {
                HashMap::new()
            };
            processes.insert(
                pid,
                ProcessInfo {
                    pid,
                    parent_pid,
                    start_ticks: 0,
                    timing: None,
                    resources: Default::default(),
                    name: "process".to_owned(),
                    environment,
                    command_line: Vec::new(),
                    executable: None,
                },
            );
        }

        let result = classify_active_pid(1, &processes);

        assert_eq!(result.steam_app_id, None);
        assert!(!result.is_game_candidate);
    }

    #[test]
    fn stops_when_a_parent_pid_repeats() {
        let processes = map([process(10, 20, "first", []), process(20, 10, "second", [])]);

        let result = classify_active_pid(10, &processes);

        assert_eq!(result, ProcessClassification::default());
    }

    #[test]
    fn identity_records_nearest_steam_id_and_exact_executable_ancestry() {
        let processes = map([
            process(10, 1, "/usr/bin/proton", [("SteamAppId", "620")]),
            process(20, 10, "/games/portal2.exe", []),
        ]);

        let identity = classify_process_identity(20, &processes);

        assert_eq!(identity.steam_app_id, Some(620));
        assert_eq!(
            identity.executable_chain,
            [
                PathBuf::from("/games/portal2.exe"),
                PathBuf::from("/usr/bin/proton")
            ]
        );
        assert!(identity.game_candidate);
    }

    #[test]
    fn identity_deduplicates_equal_executable_paths_in_nearest_first_order() {
        let processes = map([
            process(10, 1, "/games/shared", []),
            process(20, 10, "/games/parent", []),
            process(30, 20, "/games/shared", []),
        ]);

        let identity = classify_process_identity(30, &processes);

        assert_eq!(
            identity.executable_chain,
            [
                PathBuf::from("/games/shared"),
                PathBuf::from("/games/parent")
            ]
        );
    }

    #[test]
    fn identity_skips_zero_ids_and_uses_the_nearest_nonzero_ancestor_id() {
        let processes = map([
            process(10, 1, "grandparent", [("SteamAppId", "570")]),
            process(20, 10, "parent", [("SteamGameId", "620")]),
            process(30, 20, "child", [("SteamAppId", "0")]),
        ]);

        let identity = classify_process_identity(30, &processes);

        assert_eq!(identity.steam_app_id, Some(620));
    }

    #[test]
    fn identity_is_recomputed_when_a_pid_is_reused() {
        let mut processes = map([process(42, 1, "/games/portal2", [("SteamAppId", "620")])]);

        let original = classify_process_identity(42, &processes);
        processes.insert(
            42,
            process(42, 1, "/usr/bin/editor", [("SteamAppId", "730")]),
        );
        let replacement = classify_process_identity(42, &processes);

        assert_eq!(original.steam_app_id, Some(620));
        assert_eq!(replacement.steam_app_id, Some(730));
        assert_eq!(
            replacement.executable_chain,
            [PathBuf::from("/usr/bin/editor")]
        );
    }

    #[test]
    fn generic_wine_identity_is_only_candidate_metadata() {
        let processes = map([process(42, 1, "/usr/bin/wine64", [])]);

        let identity = classify_process_identity(42, &processes);

        assert_eq!(identity.steam_app_id, None);
        assert_eq!(
            identity.executable_chain,
            [PathBuf::from("/usr/bin/wine64")]
        );
        assert!(identity.game_candidate);
    }

    #[test]
    fn identity_executable_chain_is_limited_to_sixty_four_ancestors() {
        let mut processes = HashMap::new();
        for pid in 1..=65 {
            let parent_pid = if pid == 65 { 0 } else { pid + 1 };
            processes.insert(
                pid,
                ProcessInfo {
                    pid,
                    parent_pid,
                    start_ticks: 0,
                    timing: None,
                    resources: Default::default(),
                    name: "process".to_owned(),
                    environment: HashMap::new(),
                    command_line: Vec::new(),
                    executable: Some(PathBuf::from(format!("/process/{pid}"))),
                },
            );
        }

        let identity = classify_process_identity(1, &processes);

        assert_eq!(identity.executable_chain.len(), MAX_ANCESTORS);
        assert_eq!(
            identity.executable_chain.first(),
            Some(&PathBuf::from("/process/1"))
        );
        assert_eq!(
            identity.executable_chain.last(),
            Some(&PathBuf::from("/process/64"))
        );
        assert!(
            !identity
                .executable_chain
                .contains(&PathBuf::from("/process/65"))
        );
    }

    #[test]
    fn identity_cycle_records_each_process_once() {
        let processes = map([
            process(10, 20, "/process/first", []),
            process(20, 10, "/process/second", []),
        ]);

        let identity = classify_process_identity(10, &processes);

        assert_eq!(
            identity.executable_chain,
            [
                PathBuf::from("/process/first"),
                PathBuf::from("/process/second")
            ]
        );
    }
}
