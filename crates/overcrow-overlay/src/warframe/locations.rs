//! Human-readable Warframe node and mission labels.
//!
//! Source tables are embedded from the WFCD worldstate data set
//! (solNodes / missionTypes). Values are English game names such as
//! `"Proteus (Neptune)"`.

use std::{collections::HashMap, sync::OnceLock};

use serde::Deserialize;

use super::{model::bound_chars, sanitize::sanitize_display};

const SOL_NODES_JSON: &str = include_str!("data/sol_nodes.json");
const MISSION_TYPES_JSON: &str = include_str!("data/mission_types.json");
const DISPLAY_MAX: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeLabel {
    pub node: String,
    pub planet: Option<String>,
}

impl NodeLabel {
    /// Prefer `Node · Planet`, fall back to node only.
    pub fn display(&self) -> String {
        match &self.planet {
            Some(planet) if !planet.is_empty() => format!("{} · {}", self.node, planet),
            _ => self.node.clone(),
        }
    }
}

#[derive(Deserialize)]
struct NamedEntry {
    value: String,
}

fn sol_nodes() -> &'static HashMap<String, NamedEntry> {
    static NODES: OnceLock<HashMap<String, NamedEntry>> = OnceLock::new();
    NODES.get_or_init(|| {
        serde_json::from_str(SOL_NODES_JSON).unwrap_or_else(|error| {
            eprintln!("OverCrow Warframe sol node table failed to load: {error}");
            HashMap::new()
        })
    })
}

fn mission_types() -> &'static HashMap<String, NamedEntry> {
    static TYPES: OnceLock<HashMap<String, NamedEntry>> = OnceLock::new();
    TYPES.get_or_init(|| {
        serde_json::from_str(MISSION_TYPES_JSON).unwrap_or_else(|error| {
            eprintln!("OverCrow Warframe mission type table failed to load: {error}");
            HashMap::new()
        })
    })
}

/// Resolve a DE node key (`SolNode17`, `MercuryHUB`, …) into node + planet.
pub fn resolve_node(code: &str) -> NodeLabel {
    let code = code.trim();
    if code.is_empty() || code == "—" {
        return NodeLabel {
            node: "—".to_owned(),
            planet: None,
        };
    }

    if let Some(entry) = sol_nodes().get(code) {
        return split_node_value(&entry.value, code);
    }

    // Event / temporary hubs (TennoCon, …) may be missing from the static table.
    if let Some(label) = event_hub_label(code) {
        return label;
    }

    // Unknown code: keep the raw identifier so we still surface something.
    NodeLabel {
        node: bound_chars(code, DISPLAY_MAX),
        planet: None,
    }
}

fn event_hub_label(code: &str) -> Option<NodeLabel> {
    let lower = code.to_ascii_lowercase();
    if lower.starts_with("tennocon") {
        return Some(NodeLabel {
            node: "TennoCon Relay".to_owned(),
            planet: None,
        });
    }
    None
}

pub fn format_node(code: &str) -> String {
    resolve_node(code).display()
}

/// Resolve a mission type key (`MT_DEFENSE`) to a short display name.
pub fn format_mission_type(code: &str) -> String {
    let code = code.trim();
    if code.is_empty() || code == "—" {
        return "—".to_owned();
    }
    if let Some(entry) = mission_types().get(code) {
        return bound_chars(&entry.value, DISPLAY_MAX);
    }
    // Fallback English labels for a few common codes if the table misses them.
    let fallback = match code {
        "MT_DEFENSE" => "Defense",
        "MT_EXTERMINATION" => "Extermination",
        "MT_CAPTURE" => "Capture",
        "MT_MOBILE_DEFENSE" => "Mobile Defense",
        "MT_SABOTAGE" => "Sabotage",
        "MT_SURVIVAL" => "Survival",
        "MT_RESCUE" => "Rescue",
        "MT_INTEL" => "Spy",
        "MT_ARTIFACT" => "Disruption",
        "MT_TERRITORY" => "Interception",
        "MT_CORRUPTION" => "Hijack",
        "MT_EVACUATION" => "Defection",
        other => other,
    };
    bound_chars(fallback, DISPLAY_MAX)
}

fn split_node_value(value: &str, fallback_code: &str) -> NodeLabel {
    // Typical form: "Proteus (Neptune)" or "Larunda Relay (Mercury)".
    if let Some((name, rest)) = value.rsplit_once(" (")
        && let Some(planet) = rest.strip_suffix(')')
        && !name.is_empty()
        && !planet.is_empty()
        && name != fallback_code
    {
        return NodeLabel {
            node: sanitize_display(name, DISPLAY_MAX),
            planet: Some(sanitize_display(planet, DISPLAY_MAX)),
        };
    }

    // Entries that are still raw codes (e.g. SolNode0) stay as-is.
    NodeLabel {
        node: sanitize_display(value, DISPLAY_MAX),
        planet: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{format_mission_type, format_node, resolve_node};

    #[test]
    fn sol_node_splits_into_node_and_planet() {
        let label = resolve_node("SolNode17");
        assert_eq!(label.node, "Proteus");
        assert_eq!(label.planet.as_deref(), Some("Neptune"));
        assert_eq!(format_node("SolNode17"), "Proteus · Neptune");
    }

    #[test]
    fn hub_nodes_resolve() {
        assert_eq!(format_node("MercuryHUB"), "Larunda Relay · Mercury");
    }

    #[test]
    fn unknown_code_is_preserved() {
        assert_eq!(format_node("TotallyUnknownNode"), "TotallyUnknownNode");
    }

    #[test]
    fn mission_types_resolve() {
        assert_eq!(format_mission_type("MT_ARTIFACT"), "Disruption");
        assert_eq!(format_mission_type("MT_DEFENSE"), "Defense");
    }
}
