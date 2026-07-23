//! Human-readable labels for Sortie / Archon / Invasions path IDs.

use std::{collections::HashMap, sync::OnceLock};

use overcrow_config::path_tail;

use super::{model::bound_chars, sanitize::sanitize_display};

const BOSSES_JSON: &str = include_str!("data/sortie_bosses.json");
const MODIFIERS_JSON: &str = include_str!("data/sortie_modifiers.json");
const ITEMS_JSON: &str = include_str!("data/item_names.json");
const FACTIONS_JSON: &str = include_str!("data/factions.json");
const DISPLAY_MAX: usize = 64;

fn string_map(json: &'static str, name: &str) -> HashMap<String, String> {
    serde_json::from_str(json).unwrap_or_else(|error| {
        eprintln!("OverCrow Warframe {name} table failed to load: {error}");
        HashMap::new()
    })
}

fn bosses() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| string_map(BOSSES_JSON, "sortie bosses"))
}

fn modifiers() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| string_map(MODIFIERS_JSON, "sortie modifiers"))
}

fn items() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| string_map(ITEMS_JSON, "item names"))
}

fn factions() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| string_map(FACTIONS_JSON, "factions"))
}

fn prettify_tail(tail: &str) -> String {
    let mut trimmed = tail.trim().to_owned();
    for prefix in ["SORTIE_MODIFIER_", "SORTIE_BOSS_"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            trimmed = rest.to_owned();
            break;
        }
    }
    if trimmed.is_empty() {
        return bound_chars(tail, DISPLAY_MAX);
    }
    let mut out = String::new();
    let mut prev_lower = false;
    for ch in trimmed.chars() {
        if ch == '_' {
            out.push(' ');
            prev_lower = false;
            continue;
        }
        if prev_lower && ch.is_ascii_uppercase() {
            out.push(' ');
        }
        out.push(ch);
        prev_lower = ch.is_ascii_lowercase();
    }
    let pretty = out
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    bound_chars(&pretty, DISPLAY_MAX)
}

fn lookup_or_pretty(map: &HashMap<String, String>, raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "—".to_owned();
    }
    if let Some(label) = map.get(raw) {
        return sanitize_display(label, DISPLAY_MAX);
    }
    let tail = path_tail(raw);
    if let Some(label) = map.get(&tail) {
        return sanitize_display(label, DISPLAY_MAX);
    }
    sanitize_display(&prettify_tail(&tail), DISPLAY_MAX)
}

pub fn format_boss(code: &str) -> String {
    lookup_or_pretty(bosses(), code)
}

/// Archon Hunt final reward is a Tauforged-capable shard whose color is tied to the boss.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchonShardHint {
    pub label: &'static str,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub fn archon_shard_hint(boss_label_or_code: &str) -> Option<ArchonShardHint> {
    let key = boss_label_or_code.to_ascii_uppercase();
    if key.contains("NIRA") {
        Some(ArchonShardHint {
            label: "Amber Archon Shard",
            r: 255,
            g: 196,
            b: 72,
        })
    } else if key.contains("BOREAL") {
        Some(ArchonShardHint {
            label: "Azure Archon Shard",
            r: 96,
            g: 168,
            b: 255,
        })
    } else if key.contains("AMAR") {
        Some(ArchonShardHint {
            label: "Crimson Archon Shard",
            r: 232,
            g: 88,
            b: 88,
        })
    } else {
        None
    }
}

pub fn format_modifier(code: &str) -> String {
    lookup_or_pretty(modifiers(), code)
}

pub fn format_item(path: &str) -> String {
    lookup_or_pretty(items(), path)
}

pub fn format_faction(code: &str) -> String {
    lookup_or_pretty(factions(), code)
}

pub fn item_key(path: &str) -> String {
    bound_chars(&path_tail(path), DISPLAY_MAX)
}

#[cfg(test)]
mod tests {
    use super::{archon_shard_hint, format_boss, format_item, format_modifier, item_key};

    #[test]
    fn known_labels_resolve() {
        assert_eq!(format_boss("SORTIE_BOSS_NIRA"), "Archon Nira");
        assert_eq!(
            format_modifier("SORTIE_MODIFIER_EXIMUS"),
            "Eximus stronghold"
        );
        assert_eq!(
            format_item("/Lotus/Types/Items/Research/EnergyComponent"),
            "Fieldron"
        );
        assert_eq!(
            item_key("/Lotus/Types/Items/Research/EnergyComponent"),
            "EnergyComponent"
        );
        assert_eq!(
            archon_shard_hint("Archon Nira").map(|s| s.label),
            Some("Amber Archon Shard")
        );
    }

    #[test]
    fn unknown_falls_back() {
        let label = format_boss("SORTIE_BOSS_UNKNOWN_XYZ");
        assert!(!label.is_empty());
        assert_ne!(label, "—");
    }
}
