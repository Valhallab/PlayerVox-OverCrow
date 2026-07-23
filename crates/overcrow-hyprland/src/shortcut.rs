use std::{error::Error, fmt};

use overcrow_config::ShortcutSettings;
use serde::Deserialize;

pub const PORTAL_SHORTCUT_NAME: &str = "com.playervox.OverCrow:toggle-overlay";
const LEGACY_PORTAL_SHORTCUT_NAME: &str = ":toggle-overlay";
pub const BIND_DESCRIPTION: &str = "OverCrow overlay";
pub const TOGGLE_MANUAL_STOPWATCH_PORTAL_ACTION: &str =
    "com.playervox.OverCrow:toggle-manual-stopwatch";
const LEGACY_TOGGLE_MANUAL_STOPWATCH_PORTAL_ACTION: &str = ":toggle-manual-stopwatch";
pub const TOGGLE_MANUAL_STOPWATCH_BIND_DESCRIPTION: &str =
    "OverCrow manual stopwatch start or pause";
pub const RESET_MANUAL_STOPWATCH_PORTAL_ACTION: &str =
    "com.playervox.OverCrow:reset-manual-stopwatch";
const LEGACY_RESET_MANUAL_STOPWATCH_PORTAL_ACTION: &str = ":reset-manual-stopwatch";
pub const RESET_MANUAL_STOPWATCH_BIND_DESCRIPTION: &str = "OverCrow manual stopwatch reset";

const META_MASK: u32 = 64;
const CTRL_MASK: u32 = 4;
const ALT_MASK: u32 = 8;
const SHIFT_MASK: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutBackend {
    Lua,
    Compatibility,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutError {
    message: &'static str,
}

impl ShortcutError {
    fn invalid_accelerator() -> Self {
        Self {
            message: "unsupported shortcut accelerator",
        }
    }
}

impl fmt::Display for ShortcutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl Error for ShortcutError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutSpec {
    keys: String,
    key: String,
    modmask: u32,
    portal_action: &'static str,
    description: &'static str,
}

impl ShortcutSpec {
    pub fn from_settings(settings: &ShortcutSettings) -> Result<Option<Self>, ShortcutError> {
        if !settings.enabled {
            return Ok(None);
        }

        Self::parse(
            &settings.accelerator,
            PORTAL_SHORTCUT_NAME,
            BIND_DESCRIPTION,
        )
        .map(Some)
    }

    pub fn manual_stopwatch() -> Result<[Self; 2], ShortcutError> {
        // Meta+Alt+S is Omarchy "move window to scratchpad" — do not use it.
        Ok([
            Self::parse(
                "Meta+Alt+P",
                TOGGLE_MANUAL_STOPWATCH_PORTAL_ACTION,
                TOGGLE_MANUAL_STOPWATCH_BIND_DESCRIPTION,
            )?,
            Self::parse(
                "Meta+Alt+R",
                RESET_MANUAL_STOPWATCH_PORTAL_ACTION,
                RESET_MANUAL_STOPWATCH_BIND_DESCRIPTION,
            )?,
        ])
    }

    fn parse(
        accelerator: &str,
        portal_action: &'static str,
        description: &'static str,
    ) -> Result<Self, ShortcutError> {
        let parts = accelerator.split('+').collect::<Vec<_>>();
        if parts.len() < 2 {
            return Err(ShortcutError::invalid_accelerator());
        }

        let (key, modifiers) = parts
            .split_last()
            .ok_or_else(ShortcutError::invalid_accelerator)?;
        let mut key_characters = key.chars();
        let Some(key_character) = key_characters.next() else {
            return Err(ShortcutError::invalid_accelerator());
        };
        if !key_character.is_ascii_alphanumeric() || key_characters.next().is_some() {
            return Err(ShortcutError::invalid_accelerator());
        }

        let mut mask = 0;
        let mut rendered = Vec::with_capacity(parts.len());
        let mut previous_rank = None;
        for modifier in modifiers {
            let (rank, name, bit) = match *modifier {
                "Meta" => (0, "SUPER", META_MASK),
                "Ctrl" => (1, "CTRL", CTRL_MASK),
                "Alt" => (2, "ALT", ALT_MASK),
                "Shift" => (3, "SHIFT", SHIFT_MASK),
                _ => return Err(ShortcutError::invalid_accelerator()),
            };
            if previous_rank.is_some_and(|previous| rank <= previous) {
                return Err(ShortcutError::invalid_accelerator());
            }
            previous_rank = Some(rank);
            mask |= bit;
            rendered.push(name);
        }

        let key = key_character.to_ascii_uppercase().to_string();
        rendered.push(&key);
        Ok(Self {
            keys: rendered.join(" + "),
            key,
            modmask: mask,
            portal_action,
            description,
        })
    }

    pub fn from_owned_binding(binding: &HyprBinding) -> Option<Self> {
        if binding.dispatcher != "global" {
            return None;
        }
        let (portal_action, description) = known_binding_identity(binding)?;

        let mut key_characters = binding.key.chars();
        let key_character = key_characters.next()?;
        if !key_character.is_ascii_alphanumeric() || key_characters.next().is_some() {
            return None;
        }

        let supported_mask = META_MASK | CTRL_MASK | ALT_MASK | SHIFT_MASK;
        if binding.modmask == 0 || binding.modmask & !supported_mask != 0 {
            return None;
        }

        let mut rendered = Vec::with_capacity(5);
        for (bit, name) in [
            (META_MASK, "SUPER"),
            (CTRL_MASK, "CTRL"),
            (ALT_MASK, "ALT"),
            (SHIFT_MASK, "SHIFT"),
        ] {
            if binding.modmask & bit != 0 {
                rendered.push(name);
            }
        }
        let key = key_character.to_ascii_uppercase().to_string();
        rendered.push(&key);
        Some(Self {
            keys: rendered.join(" + "),
            key,
            modmask: binding.modmask,
            portal_action,
            description,
        })
    }

    pub fn keys(&self) -> &str {
        &self.keys
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn modmask(&self) -> u32 {
        self.modmask
    }

    pub fn portal_action(&self) -> &str {
        self.portal_action
    }

    pub fn description(&self) -> &str {
        self.description
    }

    pub fn bind_request(&self, backend: ShortcutBackend) -> String {
        self.bind_request_for_action(backend, self.portal_action)
    }

    fn bind_request_for_action(&self, backend: ShortcutBackend, portal_action: &str) -> String {
        match backend {
            ShortcutBackend::Lua => format!(
                "eval hl.bind(\"{}\", hl.dsp.global(\"{}\"), {{ description = \"{}\" }})",
                self.keys, portal_action, self.description
            ),
            ShortcutBackend::Compatibility => format!(
                "keyword bindd {}, {}, {}, global, {}",
                self.compatibility_modifiers(),
                self.key,
                self.description,
                portal_action,
            ),
        }
    }

    pub fn unbind_request(&self, backend: ShortcutBackend) -> String {
        match backend {
            ShortcutBackend::Lua => format!("eval hl.unbind(\"{}\")", self.keys),
            ShortcutBackend::Compatibility => format!(
                "keyword unbind {}, {}",
                self.compatibility_modifiers(),
                self.key
            ),
        }
    }

    pub fn matches_accelerator(&self, binding: &HyprBinding) -> bool {
        binding.modmask == self.modmask && binding.key == self.key
    }

    pub fn owns(&self, binding: &HyprBinding) -> bool {
        self.owns_action(binding, self.portal_action)
            || legacy_portal_action(self.portal_action)
                .is_some_and(|action| self.owns_action(binding, action))
    }

    fn owns_action(&self, binding: &HyprBinding, portal_action: &str) -> bool {
        self.matches_accelerator(binding)
            && binding.description == self.description
            && binding.dispatcher == "global"
            && binding.arg == portal_action
    }

    fn advertised_portal_action(&self, globals: &[GlobalShortcut]) -> Option<&'static str> {
        if globals
            .iter()
            .any(|shortcut| shortcut.name == self.portal_action)
        {
            return Some(self.portal_action);
        }
        legacy_portal_action(self.portal_action)
            .filter(|action| globals.iter().any(|shortcut| shortcut.name == *action))
    }

    fn compatibility_modifiers(&self) -> String {
        [
            (META_MASK, "SUPER"),
            (CTRL_MASK, "CTRL"),
            (ALT_MASK, "ALT"),
            (SHIFT_MASK, "SHIFT"),
        ]
        .into_iter()
        .filter_map(|(bit, name)| (self.modmask & bit != 0).then_some(name))
        .collect::<Vec<_>>()
        .join(" ")
    }
}

fn known_binding_identity(binding: &HyprBinding) -> Option<(&'static str, &'static str)> {
    [
        (
            PORTAL_SHORTCUT_NAME,
            LEGACY_PORTAL_SHORTCUT_NAME,
            BIND_DESCRIPTION,
        ),
        (
            TOGGLE_MANUAL_STOPWATCH_PORTAL_ACTION,
            LEGACY_TOGGLE_MANUAL_STOPWATCH_PORTAL_ACTION,
            TOGGLE_MANUAL_STOPWATCH_BIND_DESCRIPTION,
        ),
        (
            RESET_MANUAL_STOPWATCH_PORTAL_ACTION,
            LEGACY_RESET_MANUAL_STOPWATCH_PORTAL_ACTION,
            RESET_MANUAL_STOPWATCH_BIND_DESCRIPTION,
        ),
    ]
    .into_iter()
    .find_map(|(action, legacy_action, description)| {
        ((binding.arg == action || binding.arg == legacy_action)
            && binding.description == description)
            .then_some((action, description))
    })
}

fn legacy_portal_action(action: &str) -> Option<&'static str> {
    match action {
        PORTAL_SHORTCUT_NAME => Some(LEGACY_PORTAL_SHORTCUT_NAME),
        TOGGLE_MANUAL_STOPWATCH_PORTAL_ACTION => Some(LEGACY_TOGGLE_MANUAL_STOPWATCH_PORTAL_ACTION),
        RESET_MANUAL_STOPWATCH_PORTAL_ACTION => Some(LEGACY_RESET_MANUAL_STOPWATCH_PORTAL_ACTION),
        _ => None,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct GlobalShortcut {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HyprBinding {
    pub modmask: u32,
    pub key: String,
    pub description: String,
    pub dispatcher: String,
    pub arg: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShortcutDecision {
    Idle,
    WaitForPortal,
    Bind { request: String },
    Unbind { request: String },
    Conflict,
    AmbiguousOwnership,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReconcileState {
    Unknown,
    Absent,
    Owned,
    Conflict,
}

#[derive(Clone, Debug)]
pub struct ShortcutReconciler {
    spec: ShortcutSpec,
    backend: ShortcutBackend,
    state: ReconcileState,
    last_desired: Option<bool>,
    warning_reported: bool,
}

impl ShortcutReconciler {
    pub fn new(spec: ShortcutSpec, backend: ShortcutBackend) -> Self {
        Self {
            spec,
            backend,
            state: ReconcileState::Unknown,
            last_desired: None,
            warning_reported: false,
        }
    }

    pub fn spec(&self) -> &ShortcutSpec {
        &self.spec
    }

    pub fn invalidate(&mut self) {
        self.state = ReconcileState::Unknown;
    }

    pub fn mark_bound(&mut self) {
        self.state = ReconcileState::Owned;
        self.warning_reported = false;
    }

    pub fn mark_unbound(&mut self) {
        self.state = ReconcileState::Absent;
        self.warning_reported = false;
    }

    pub fn is_owned(&self) -> bool {
        self.state == ReconcileState::Owned
    }

    pub fn take_warning(&mut self) -> bool {
        if self.state != ReconcileState::Conflict || self.warning_reported {
            return false;
        }
        self.warning_reported = true;
        true
    }

    pub fn needs_probe(&mut self, desired: bool) -> bool {
        self.observe_desired(desired);
        !matches!(
            (desired, self.state),
            (true, ReconcileState::Owned | ReconcileState::Conflict)
                | (false, ReconcileState::Absent | ReconcileState::Conflict)
        )
    }

    pub fn plan(
        &mut self,
        desired: bool,
        globals: &[GlobalShortcut],
        bindings: &[HyprBinding],
    ) -> ShortcutDecision {
        let advertised_action = self.spec.advertised_portal_action(globals);
        let effective_desired = desired && advertised_action.is_some();
        self.observe_desired(effective_desired);
        if effective_desired && self.state == ReconcileState::Conflict {
            return ShortcutDecision::Conflict;
        }

        let matching = bindings
            .iter()
            .filter(|binding| self.spec.matches_accelerator(binding))
            .collect::<Vec<_>>();
        let owned_count = matching
            .iter()
            .filter(|binding| self.spec.owns(binding))
            .count();
        let foreign_count = matching.len().saturating_sub(owned_count);

        if !effective_desired {
            return match (owned_count, foreign_count) {
                (0, _) => {
                    self.state = ReconcileState::Absent;
                    self.warning_reported = false;
                    if desired {
                        ShortcutDecision::WaitForPortal
                    } else {
                        ShortcutDecision::Idle
                    }
                }
                (1, 0) => {
                    self.state = ReconcileState::Owned;
                    self.warning_reported = false;
                    ShortcutDecision::Unbind {
                        request: self.spec.unbind_request(self.backend),
                    }
                }
                _ => {
                    self.state = ReconcileState::Conflict;
                    ShortcutDecision::AmbiguousOwnership
                }
            };
        }

        let Some(advertised_action) = advertised_action else {
            self.state = ReconcileState::Absent;
            return ShortcutDecision::WaitForPortal;
        };
        let desired_owned_count = matching
            .iter()
            .filter(|binding| self.spec.owns_action(binding, advertised_action))
            .count();
        if desired_owned_count == 1 && owned_count == 1 && foreign_count == 0 {
            self.state = ReconcileState::Owned;
            self.warning_reported = false;
            return ShortcutDecision::Idle;
        }
        if desired_owned_count == 0 && owned_count == 1 && foreign_count == 0 {
            self.state = ReconcileState::Owned;
            self.warning_reported = false;
            return ShortcutDecision::Unbind {
                request: self.spec.unbind_request(self.backend),
            };
        }
        if !matching.is_empty() {
            self.state = ReconcileState::Conflict;
            return ShortcutDecision::Conflict;
        }
        self.warning_reported = false;
        ShortcutDecision::Bind {
            request: self
                .spec
                .bind_request_for_action(self.backend, advertised_action),
        }
    }

    fn observe_desired(&mut self, desired: bool) {
        if self.last_desired != Some(desired) {
            self.last_desired = Some(desired);
            self.state = ReconcileState::Unknown;
            self.warning_reported = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use overcrow_config::ShortcutSettings;

    use super::{
        BIND_DESCRIPTION, GlobalShortcut, HyprBinding, PORTAL_SHORTCUT_NAME,
        RESET_MANUAL_STOPWATCH_PORTAL_ACTION, ShortcutBackend, ShortcutDecision,
        ShortcutReconciler, ShortcutSpec, TOGGLE_MANUAL_STOPWATCH_PORTAL_ACTION,
    };

    fn settings(enabled: bool, accelerator: &str) -> ShortcutSettings {
        ShortcutSettings {
            enabled,
            accelerator: accelerator.to_owned(),
        }
    }

    fn default_spec() -> ShortcutSpec {
        ShortcutSpec::from_settings(&settings(true, "Meta+Alt+O"))
            .expect("valid shortcut")
            .expect("enabled shortcut")
    }

    #[test]
    fn portal_dispatch_names_include_the_registered_application_id() {
        assert_eq!(
            PORTAL_SHORTCUT_NAME,
            "com.playervox.OverCrow:toggle-overlay"
        );
        assert_eq!(
            TOGGLE_MANUAL_STOPWATCH_PORTAL_ACTION,
            "com.playervox.OverCrow:toggle-manual-stopwatch"
        );
        assert_eq!(
            RESET_MANUAL_STOPWATCH_PORTAL_ACTION,
            "com.playervox.OverCrow:reset-manual-stopwatch"
        );
    }

    fn portal_action() -> GlobalShortcut {
        GlobalShortcut {
            name: PORTAL_SHORTCUT_NAME.to_owned(),
            description: "Open or close the OverCrow overlay".to_owned(),
        }
    }

    fn legacy_portal_action() -> GlobalShortcut {
        GlobalShortcut {
            name: ":toggle-overlay".to_owned(),
            description: "Open or close the OverCrow overlay".to_owned(),
        }
    }

    fn manual_specs() -> Vec<ShortcutSpec> {
        ShortcutSpec::manual_stopwatch()
            .expect("fixed manual shortcut specs are valid")
            .into_iter()
            .collect()
    }

    fn binding(description: &str, dispatcher: &str, arg: &str) -> HyprBinding {
        HyprBinding {
            modmask: 72,
            key: "O".to_owned(),
            description: description.to_owned(),
            dispatcher: dispatcher.to_owned(),
            arg: arg.to_owned(),
        }
    }

    fn owned_binding() -> HyprBinding {
        binding(BIND_DESCRIPTION, "global", PORTAL_SHORTCUT_NAME)
    }

    #[test]
    fn normalizes_the_validated_accelerator_for_hyprland() {
        let spec = ShortcutSpec::from_settings(&settings(true, "Meta+Ctrl+Alt+Shift+9"))
            .expect("valid shortcut")
            .expect("enabled shortcut");

        assert_eq!(spec.keys(), "SUPER + CTRL + ALT + SHIFT + 9");
        assert_eq!(spec.modmask(), 64 | 4 | 8 | 1);
        assert_eq!(spec.key(), "9");
    }

    #[test]
    fn fixed_manual_actions_use_the_canonical_parser_and_exact_identity() {
        let specs = manual_specs();

        assert_eq!(
            specs
                .iter()
                .map(|spec| (spec.keys(), spec.portal_action(), spec.description()))
                .collect::<Vec<_>>(),
            [
                (
                    "SUPER + ALT + P",
                    "com.playervox.OverCrow:toggle-manual-stopwatch",
                    "OverCrow manual stopwatch start or pause",
                ),
                (
                    "SUPER + ALT + R",
                    "com.playervox.OverCrow:reset-manual-stopwatch",
                    "OverCrow manual stopwatch reset",
                ),
            ]
        );
    }

    #[test]
    fn disabled_shortcuts_have_no_spec() {
        assert_eq!(
            ShortcutSpec::from_settings(&settings(false, "Meta+Alt+O"))
                .expect("valid disabled shortcut"),
            None
        );
    }

    #[test]
    fn malformed_or_noncanonical_accelerators_are_rejected() {
        for accelerator in [
            "",
            "O",
            "Super+O",
            "Alt+Meta+O",
            "Meta+Alt+Alt+O",
            "Meta+F1",
            "Meta+é",
            "Meta++O",
        ] {
            assert!(
                ShortcutSpec::from_settings(&settings(true, accelerator)).is_err(),
                "accepted {accelerator:?}"
            );
        }
    }

    #[test]
    fn lua_requests_are_exact_and_data_independent() {
        let spec = default_spec();

        assert_eq!(
            spec.bind_request(ShortcutBackend::Lua),
            "eval hl.bind(\"SUPER + ALT + O\", hl.dsp.global(\"com.playervox.OverCrow:toggle-overlay\"), { description = \"OverCrow overlay\" })"
        );
        assert_eq!(
            spec.unbind_request(ShortcutBackend::Lua),
            "eval hl.unbind(\"SUPER + ALT + O\")"
        );
        assert_eq!(
            spec.bind_request(ShortcutBackend::Compatibility),
            "keyword bindd SUPER ALT, O, OverCrow overlay, global, com.playervox.OverCrow:toggle-overlay"
        );
        assert_eq!(
            spec.unbind_request(ShortcutBackend::Compatibility),
            "keyword unbind SUPER ALT, O"
        );
    }

    #[test]
    fn acquires_only_after_the_portal_action_exists() {
        let mut reconciler =
            ShortcutReconciler::new(default_spec(), ShortcutBackend::Compatibility);

        assert_eq!(
            reconciler.plan(true, &[], &[]),
            ShortcutDecision::WaitForPortal
        );
        assert_eq!(
            reconciler.plan(true, &[portal_action()], &[]),
            ShortcutDecision::Bind {
                request: default_spec().bind_request(ShortcutBackend::Compatibility)
            }
        );
    }

    #[test]
    fn advertised_legacy_portal_action_remains_usable() {
        let mut reconciler =
            ShortcutReconciler::new(default_spec(), ShortcutBackend::Compatibility);

        assert_eq!(
            reconciler.plan(true, &[legacy_portal_action()], &[]),
            ShortcutDecision::Bind {
                request: "keyword bindd SUPER ALT, O, OverCrow overlay, global, :toggle-overlay"
                    .to_owned()
            }
        );
    }

    #[test]
    fn one_exact_legacy_binding_is_removed_before_scoped_replacement() {
        let legacy = binding(BIND_DESCRIPTION, "global", ":toggle-overlay");
        let spec = default_spec();
        let mut reconciler = ShortcutReconciler::new(spec.clone(), ShortcutBackend::Compatibility);

        assert!(ShortcutSpec::from_owned_binding(&legacy).is_some());
        assert_eq!(
            reconciler.plan(true, &[portal_action()], &[legacy]),
            ShortcutDecision::Unbind {
                request: spec.unbind_request(ShortcutBackend::Compatibility)
            }
        );
        reconciler.mark_unbound();
        assert_eq!(
            reconciler.plan(true, &[portal_action()], &[]),
            ShortcutDecision::Bind {
                request: spec.bind_request(ShortcutBackend::Compatibility)
            }
        );
    }

    #[test]
    fn manual_key_waits_for_its_exact_portal_action() {
        let spec = manual_specs().remove(0);
        let mut reconciler = ShortcutReconciler::new(spec.clone(), ShortcutBackend::Compatibility);

        assert_eq!(
            reconciler.plan(true, &[portal_action()], &[]),
            ShortcutDecision::WaitForPortal
        );
        assert_eq!(
            reconciler.plan(
                true,
                &[GlobalShortcut {
                    name: spec.portal_action().to_owned(),
                    description: "Start or pause the OverCrow manual stopwatch".to_owned(),
                }],
                &[],
            ),
            ShortcutDecision::Bind {
                request: spec.bind_request(ShortcutBackend::Compatibility)
            }
        );
    }

    #[test]
    fn disappearing_portal_action_releases_only_its_owned_key() {
        let spec = manual_specs().remove(0);
        let owned = HyprBinding {
            modmask: spec.modmask(),
            key: spec.key().to_owned(),
            description: spec.description().to_owned(),
            dispatcher: "global".to_owned(),
            arg: spec.portal_action().to_owned(),
        };
        let mut reconciler = ShortcutReconciler::new(spec.clone(), ShortcutBackend::Lua);
        reconciler.mark_bound();
        reconciler.invalidate();

        assert_eq!(
            reconciler.plan(true, &[], &[owned]),
            ShortcutDecision::Unbind {
                request: spec.unbind_request(ShortcutBackend::Lua)
            }
        );
    }

    #[test]
    fn successful_bind_is_stable_until_invalidated() {
        let mut reconciler = ShortcutReconciler::new(default_spec(), ShortcutBackend::Lua);
        assert!(matches!(
            reconciler.plan(true, &[portal_action()], &[]),
            ShortcutDecision::Bind { .. }
        ));
        reconciler.mark_bound();

        assert_eq!(
            reconciler.plan(true, &[portal_action()], &[owned_binding()]),
            ShortcutDecision::Idle
        );
        reconciler.invalidate();
        assert_eq!(
            reconciler.plan(true, &[portal_action()], &[owned_binding()]),
            ShortcutDecision::Idle
        );
    }

    #[test]
    fn bridge_restart_adopts_one_exact_owned_binding() {
        let mut reconciler = ShortcutReconciler::new(default_spec(), ShortcutBackend::Lua);

        assert_eq!(
            reconciler.plan(true, &[portal_action()], &[owned_binding()]),
            ShortcutDecision::Idle
        );
        assert!(reconciler.is_owned());
    }

    #[test]
    fn foreign_binding_conflict_fails_closed_until_invalidated() {
        let foreign = binding("Obsidian", "exec", "obsidian");
        let mut reconciler = ShortcutReconciler::new(default_spec(), ShortcutBackend::Lua);

        assert_eq!(
            reconciler.plan(true, &[portal_action()], std::slice::from_ref(&foreign)),
            ShortcutDecision::Conflict
        );
        assert_eq!(
            reconciler.plan(true, &[portal_action()], &[]),
            ShortcutDecision::Conflict
        );

        reconciler.invalidate();
        assert!(matches!(
            reconciler.plan(true, &[portal_action()], &[]),
            ShortcutDecision::Bind { .. }
        ));
    }

    #[test]
    fn conflict_warning_is_reported_once_across_periodic_invalidations() {
        let foreign = binding("Obsidian", "exec", "obsidian");
        let mut reconciler = ShortcutReconciler::new(default_spec(), ShortcutBackend::Lua);

        assert_eq!(
            reconciler.plan(true, &[portal_action()], std::slice::from_ref(&foreign)),
            ShortcutDecision::Conflict
        );
        assert!(reconciler.take_warning());
        assert!(!reconciler.take_warning());

        reconciler.invalidate();
        assert_eq!(
            reconciler.plan(true, &[portal_action()], std::slice::from_ref(&foreign)),
            ShortcutDecision::Conflict
        );
        assert!(!reconciler.take_warning());

        reconciler.invalidate();
        assert!(matches!(
            reconciler.plan(true, &[portal_action()], &[]),
            ShortcutDecision::Bind { .. }
        ));
        reconciler.mark_bound();
        reconciler.invalidate();
        assert_eq!(
            reconciler.plan(true, &[portal_action()], &[foreign]),
            ShortcutDecision::Conflict
        );
        assert!(reconciler.take_warning());
    }

    #[test]
    fn release_requires_one_unambiguous_owned_binding() {
        let mut reconciler =
            ShortcutReconciler::new(default_spec(), ShortcutBackend::Compatibility);
        reconciler.mark_bound();
        let owned = owned_binding();

        assert_eq!(
            reconciler.plan(false, &[], std::slice::from_ref(&owned)),
            ShortcutDecision::Unbind {
                request: default_spec().unbind_request(ShortcutBackend::Compatibility)
            }
        );

        reconciler.mark_bound();
        let foreign = binding("Foreign action", "exec", "foreign");
        assert_eq!(
            reconciler.plan(false, &[], &[owned, foreign]),
            ShortcutDecision::AmbiguousOwnership
        );
    }

    #[test]
    fn release_adopts_and_removes_a_stale_exact_binding() {
        let mut reconciler = ShortcutReconciler::new(default_spec(), ShortcutBackend::Lua);

        assert!(matches!(
            reconciler.plan(false, &[], &[owned_binding()]),
            ShortcutDecision::Unbind { .. }
        ));
    }

    #[test]
    fn release_without_an_owned_binding_is_idle() {
        let mut reconciler = ShortcutReconciler::new(default_spec(), ShortcutBackend::Lua);

        assert_eq!(reconciler.plan(false, &[], &[]), ShortcutDecision::Idle);
        assert!(!reconciler.is_owned());
    }

    #[test]
    fn decodes_current_global_shortcut_json_without_extra_authority() {
        let globals: Vec<GlobalShortcut> = serde_json::from_str(
            r#"[{"name":"com.playervox.OverCrow:toggle-overlay","description":"Open or close the OverCrow overlay"}]"#,
        )
        .unwrap();

        assert_eq!(globals, vec![portal_action()]);
    }
}
