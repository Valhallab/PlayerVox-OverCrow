use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::PermissionsExt,
    },
    path::{Path, PathBuf},
};

use crate::{ControlModel, LifecycleStatus};

pub(crate) const MAX_SESSION_TYPE_BYTES: usize = 32;
pub(crate) const MAX_DESKTOP_METADATA_BYTES: usize = 128;
pub(crate) const MAX_CONFIG_PATH_BYTES: usize = 256;
pub(crate) const MAX_RAW_PATH_BYTES: usize = 4 * 1024;
pub(crate) const MAX_PATH_ENTRIES: usize = 32;
pub(crate) const MAX_DIAGNOSTIC_LABEL_BYTES: usize = 64;
pub(crate) const MAX_DIAGNOSTIC_DETAIL_BYTES: usize = 512;
pub(crate) const MAX_DIAGNOSTIC_COUNT: usize = 999_999_999;
pub(crate) const MAX_SETTINGS_WARNINGS: usize = 1;
pub(crate) const MAX_DISCOVERY_WARNINGS: usize = 16;
pub(crate) const MAX_WARNING_BYTES: usize = 255;
pub(crate) const MAX_SOURCE_WARNING_AGGREGATE_BYTES: usize = 1024;
pub(crate) const MAX_RENDERED_WARNING_AGGREGATE_BYTES: usize = 1536;
pub(crate) const MAX_PATH_DISPLAY_BYTES: usize = 320;
const TRUNCATION_LABEL: &str = "Diagnostic bounds";
const TRUNCATION_DETAIL: &str =
    "Some diagnostic input was truncated to bounded limits; discarded content is not shown.";
const PORTAL_EXECUTABLES: [&str; 1] = ["xdg-desktop-portal"];
const PORTAL_BACKEND_EXECUTABLES: [&str; 7] = [
    "xdg-desktop-portal-gtk",
    "xdg-desktop-portal-kde",
    "xdg-desktop-portal-hyprland",
    "xdg-desktop-portal-wlr",
    "xdg-desktop-portal-gnome",
    "xdg-desktop-portal-lxqt",
    "xdg-desktop-portal-xapp",
];
#[cfg(test)]
pub(crate) const MAX_EXECUTABLE_METADATA_CHECKS: usize =
    MAX_PATH_ENTRIES * (PORTAL_EXECUTABLES.len() + PORTAL_BACKEND_EXECUTABLES.len());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Ok,
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticItem {
    pub label: String,
    pub detail: String,
    pub level: Level,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticReport {
    pub lifecycle_state: &'static str,
    pub items: Vec<DiagnosticItem>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Availability {
    Available,
    Unavailable,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PortalPickerInput {
    pub portal_executable: Availability,
    pub backend_executable: Availability,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticInput {
    pub session_type: Option<String>,
    pub current_desktop: Option<String>,
    pub desktop_session: Option<String>,
    pub home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub portal_picker: PortalPickerInput,
    pub settings_warning: Option<String>,
    pub discovery_warnings: Vec<String>,
    pub discovered_steam_games: usize,
    pub selected_steam_games: usize,
    pub selected_manual_games: usize,
    pub lifecycle_status: LifecycleStatus,
    pub environment_was_truncated: bool,
    pub model_was_truncated: bool,
}

impl DiagnosticInput {
    /// Captures the bounded, read-only process metadata used by foundation diagnostics.
    pub fn from_current_process() -> Self {
        diagnostic_input_from_environment_with(|name| env::var_os(name), is_executable_file)
    }

    pub(crate) fn normalize(mut self) -> Self {
        let mut environment_was_truncated = self.environment_was_truncated;
        self.session_type = self.session_type.take().map(|value| {
            let (value, was_truncated) = bounded_owned_text(value, MAX_SESSION_TYPE_BYTES);
            environment_was_truncated |= was_truncated;
            value
        });
        self.current_desktop = self.current_desktop.take().map(|value| {
            let (value, was_truncated) = bounded_owned_text(value, MAX_DESKTOP_METADATA_BYTES);
            environment_was_truncated |= was_truncated;
            value
        });
        self.desktop_session = self.desktop_session.take().map(|value| {
            let (value, was_truncated) = bounded_owned_text(value, MAX_DESKTOP_METADATA_BYTES);
            environment_was_truncated |= was_truncated;
            value
        });
        self.home = normalized_config_path(self.home.take(), &mut environment_was_truncated);
        self.xdg_config_home =
            normalized_config_path(self.xdg_config_home.take(), &mut environment_was_truncated);
        self.environment_was_truncated = environment_was_truncated;

        let (settings_warning, discovery_warnings, warnings_were_truncated) =
            bounded_owned_warning_sources(
                self.settings_warning.take(),
                std::mem::take(&mut self.discovery_warnings),
            );
        self.settings_warning = settings_warning;
        self.discovery_warnings = discovery_warnings;
        let counts_were_truncated = self.discovered_steam_games > MAX_DIAGNOSTIC_COUNT
            || self.selected_steam_games > MAX_DIAGNOSTIC_COUNT
            || self.selected_manual_games > MAX_DIAGNOSTIC_COUNT;
        self.discovered_steam_games = self.discovered_steam_games.min(MAX_DIAGNOSTIC_COUNT);
        self.selected_steam_games = self.selected_steam_games.min(MAX_DIAGNOSTIC_COUNT);
        self.selected_manual_games = self.selected_manual_games.min(MAX_DIAGNOSTIC_COUNT);
        self.model_was_truncated |= warnings_were_truncated || counts_were_truncated;
        self
    }

    /// Replaces only the model-derived part of an already captured diagnostic snapshot.
    pub fn sync_model(&mut self, model: &ControlModel) {
        let (settings_warning, discovery_warnings, was_truncated) = bounded_warning_sources(
            model.settings_warning.as_deref(),
            model.discovery_warnings.iter().map(String::as_str),
        );
        self.settings_warning = settings_warning;
        self.discovery_warnings = discovery_warnings;
        let counts_were_truncated = model.games.len() > MAX_DIAGNOSTIC_COUNT
            || model.settings.selected_steam_app_ids.len() > MAX_DIAGNOSTIC_COUNT
            || model.settings.manual_games.len() > MAX_DIAGNOSTIC_COUNT;
        self.model_was_truncated |= was_truncated || counts_were_truncated;
        self.discovered_steam_games = model.games.len().min(MAX_DIAGNOSTIC_COUNT);
        self.selected_steam_games = model
            .settings
            .selected_steam_app_ids
            .len()
            .min(MAX_DIAGNOSTIC_COUNT);
        self.selected_manual_games = model.settings.manual_games.len().min(MAX_DIAGNOSTIC_COUNT);
        self.lifecycle_status = if model.settings.enabled {
            LifecycleStatus::Enabled
        } else {
            LifecycleStatus::Disabled
        };
    }

    pub(crate) fn set_lifecycle_status(&mut self, status: LifecycleStatus) {
        self.lifecycle_status = status;
    }
}

pub(crate) fn diagnostic_input_from_environment_with(
    mut variable: impl FnMut(&str) -> Option<OsString>,
    mut executable_file: impl FnMut(&Path) -> bool,
) -> DiagnosticInput {
    let mut environment_was_truncated = false;
    let session_type = bounded_environment_text(
        variable("XDG_SESSION_TYPE"),
        MAX_SESSION_TYPE_BYTES,
        &mut environment_was_truncated,
    );
    let current_desktop = bounded_environment_text(
        variable("XDG_CURRENT_DESKTOP"),
        MAX_DESKTOP_METADATA_BYTES,
        &mut environment_was_truncated,
    );
    let desktop_session = bounded_environment_text(
        variable("DESKTOP_SESSION"),
        MAX_DESKTOP_METADATA_BYTES,
        &mut environment_was_truncated,
    );
    let home = bounded_environment_path(variable("HOME"), &mut environment_was_truncated);
    let xdg_config_home =
        bounded_environment_path(variable("XDG_CONFIG_HOME"), &mut environment_was_truncated);
    let portal_picker = match variable("PATH") {
        Some(path) => {
            let (path, path_was_truncated) = bounded_path_value(path);
            environment_was_truncated |= path_was_truncated;
            let mut entries = env::split_paths(&path)
                .take(MAX_PATH_ENTRIES + 1)
                .collect::<Vec<_>>();
            if entries.len() > MAX_PATH_ENTRIES {
                entries.truncate(MAX_PATH_ENTRIES);
                environment_was_truncated = true;
            }
            let directories = entries
                .into_iter()
                .filter(|path| path.is_absolute())
                .collect::<Vec<_>>();
            PortalPickerInput {
                portal_executable: executable_availability(
                    &directories,
                    &PORTAL_EXECUTABLES,
                    &mut executable_file,
                ),
                backend_executable: executable_availability(
                    &directories,
                    &PORTAL_BACKEND_EXECUTABLES,
                    &mut executable_file,
                ),
            }
        }
        None => PortalPickerInput::default(),
    };

    DiagnosticInput {
        session_type,
        current_desktop,
        desktop_session,
        home,
        xdg_config_home,
        portal_picker,
        environment_was_truncated,
        ..DiagnosticInput::default()
    }
    .normalize()
}

pub fn collect_foundation_diagnostics(input: DiagnosticInput) -> DiagnosticReport {
    let input = input.normalize();
    collect_normalized_foundation_diagnostics(&input)
}

pub(crate) fn collect_normalized_foundation_diagnostics(
    input: &DiagnosticInput,
) -> DiagnosticReport {
    let mut was_truncated = input.environment_was_truncated || input.model_was_truncated;

    let mut items = vec![
        desktop_session_item(
            input.session_type.as_deref(),
            input.current_desktop.as_deref(),
            input.desktop_session.as_deref(),
        ),
        settings_path_item(input, &mut was_truncated),
        portal_picker_item(input.portal_picker),
        settings_item(input.settings_warning.clone()),
        steam_discovery_item(input.discovered_steam_games),
    ];
    for warning in &input.discovery_warnings {
        items.push(DiagnosticItem {
            label: "Steam discovery warning".to_owned(),
            detail: warning.clone(),
            level: Level::Warning,
        });
    }
    items.push(DiagnosticItem {
        label: "Game selections".to_owned(),
        detail: format!(
            "{} Steam and {} manual selection(s).",
            input.selected_steam_games, input.selected_manual_games
        ),
        level: Level::Info,
    });

    DiagnosticReport {
        lifecycle_state: input.lifecycle_status.label(),
        items: finalize_items(items, was_truncated),
    }
}

fn desktop_session_item(
    session_type: Option<&str>,
    current_desktop: Option<&str>,
    desktop_session: Option<&str>,
) -> DiagnosticItem {
    let session_type = session_type
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match session_type {
        Some(value) if value.eq_ignore_ascii_case("wayland") => {
            match desktop_kind(current_desktop, desktop_session) {
                Some(DesktopKind::Hyprland) => {
                    diagnostic("Desktop session", "Wayland — Hyprland detected.", Level::Ok)
                }
                Some(DesktopKind::Plasma) => diagnostic(
                    "Desktop session",
                    "Wayland — Plasma/KDE detected.",
                    Level::Ok,
                ),
                None => diagnostic(
                    "Desktop session",
                    "Wayland — compositor not identified by desktop metadata.",
                    Level::Info,
                ),
            }
        }
        Some(value) if value.eq_ignore_ascii_case("x11") => {
            diagnostic("Desktop session", "X11 detected.", Level::Ok)
        }
        _ => diagnostic(
            "Desktop session",
            "Unknown — XDG session metadata does not identify Wayland or X11.",
            Level::Info,
        ),
    }
}

#[derive(Clone, Copy)]
enum DesktopKind {
    Hyprland,
    Plasma,
}

fn desktop_kind(
    current_desktop: Option<&str>,
    desktop_session: Option<&str>,
) -> Option<DesktopKind> {
    current_desktop
        .into_iter()
        .flat_map(|desktop| desktop.split(':'))
        .find_map(classify_desktop_token)
        .or_else(|| desktop_session.and_then(classify_desktop_token))
}

fn classify_desktop_token(token: &str) -> Option<DesktopKind> {
    let token = token.trim();
    if token.eq_ignore_ascii_case("hyprland") {
        return Some(DesktopKind::Hyprland);
    }
    [
        "kde",
        "plasma",
        "plasmawayland",
        "plasma-wayland",
        "plasma6",
        "kde-plasma",
        "kde-plasma-wayland",
    ]
    .iter()
    .any(|known| token.eq_ignore_ascii_case(known))
    .then_some(DesktopKind::Plasma)
}

fn settings_path_item(input: &DiagnosticInput, was_truncated: &mut bool) -> DiagnosticItem {
    let xdg_exceeds_limit = input
        .xdg_config_home
        .as_deref()
        .is_some_and(path_exceeds_diagnostic_limit);
    let home_exceeds_limit = input
        .home
        .as_deref()
        .is_some_and(path_exceeds_diagnostic_limit);
    *was_truncated |= xdg_exceeds_limit || home_exceeds_limit;

    if xdg_exceeds_limit
        && input
            .xdg_config_home
            .as_deref()
            .is_some_and(Path::is_absolute)
    {
        *was_truncated = true;
        return diagnostic(
            "Settings path",
            "Absolute XDG_CONFIG_HOME metadata exceeds the diagnostic display limit.",
            Level::Warning,
        );
    }
    let xdg = absolute_path(input.xdg_config_home.as_deref());
    let xdg = xdg.filter(|path| !path_exceeds_diagnostic_limit(path));
    if home_exceeds_limit && input.home.as_deref().is_some_and(Path::is_absolute) {
        *was_truncated = true;
        if xdg.is_none() {
            return diagnostic(
                "Settings path",
                "Absolute HOME metadata exceeds the diagnostic display limit.",
                Level::Warning,
            );
        }
    }
    let home =
        absolute_path(input.home.as_deref()).filter(|path| !path_exceeds_diagnostic_limit(path));
    let invalid_xdg = input
        .xdg_config_home
        .as_deref()
        .is_some_and(|path| !path.as_os_str().is_empty() && !path.is_absolute());
    let invalid_home = input
        .home
        .as_deref()
        .is_some_and(|path| !path.as_os_str().is_empty() && !path.is_absolute());

    if let Some(root) = xdg {
        let (path, display_was_truncated) =
            bounded_path_display(&root.join("overcrow/settings.json"), MAX_PATH_DISPLAY_BYTES);
        *was_truncated |= display_was_truncated;
        return diagnostic("Settings path", format!("Using {path}."), Level::Ok);
    }
    if let Some(home) = home {
        let path = home.join(".config/overcrow/settings.json");
        let (path, display_was_truncated) = bounded_path_display(&path, MAX_PATH_DISPLAY_BYTES);
        *was_truncated |= display_was_truncated;
        if invalid_xdg {
            return diagnostic(
                "Settings path",
                format!("Ignoring relative XDG_CONFIG_HOME; using {path}."),
                Level::Warning,
            );
        }
        return diagnostic("Settings path", format!("Using {path}."), Level::Ok);
    }

    let detail = if invalid_xdg || invalid_home {
        "Settings path unavailable because HOME/XDG_CONFIG_HOME metadata is relative."
    } else {
        "Settings path unavailable because no absolute HOME or XDG_CONFIG_HOME is present."
    };
    diagnostic("Settings path", detail, Level::Warning)
}

fn portal_picker_item(input: PortalPickerInput) -> DiagnosticItem {
    match (input.portal_executable, input.backend_executable) {
        (Availability::Available, Availability::Available) => diagnostic(
            "Portal picker",
            "Portal and backend executables found in bounded PATH metadata; active portal service was not queried.",
            Level::Ok,
        ),
        (Availability::Unavailable, _) | (_, Availability::Unavailable) => diagnostic(
            "Portal picker",
            "A portal or backend executable was not found in bounded PATH metadata; active portal service was not queried.",
            Level::Warning,
        ),
        _ => diagnostic(
            "Portal picker",
            "Executable availability is unknown; active portal service was not queried.",
            Level::Info,
        ),
    }
}

fn settings_item(warning: Option<String>) -> DiagnosticItem {
    match warning {
        Some(warning) => diagnostic("Lifecycle settings", warning, Level::Warning),
        None => diagnostic(
            "Lifecycle settings",
            "Loaded without a settings warning.",
            Level::Ok,
        ),
    }
}

fn steam_discovery_item(discovered_games: usize) -> DiagnosticItem {
    if discovered_games == 0 {
        diagnostic(
            "Steam discovery",
            "No Steam games are present in the current discovery snapshot.",
            Level::Info,
        )
    } else {
        diagnostic(
            "Steam discovery",
            format!("{discovered_games} Steam games are present in the current snapshot."),
            Level::Ok,
        )
    }
}

fn bounded_environment_text(
    value: Option<OsString>,
    limit: usize,
    was_truncated: &mut bool,
) -> Option<String> {
    value.map(|value| {
        if let Some(text) = value.to_str() {
            let (text, text_was_truncated) = bounded_text(text, limit);
            *was_truncated |= text_was_truncated;
            return text;
        }

        let bytes = value.as_bytes();
        let prefix = &bytes[..bytes.len().min(limit)];
        let lossy = String::from_utf8_lossy(prefix);
        let (text, text_was_truncated) = bounded_text(&lossy, limit);
        *was_truncated |= bytes.len() > limit || text_was_truncated || value.to_str().is_none();
        text
    })
}

fn bounded_environment_path(value: Option<OsString>, was_truncated: &mut bool) -> Option<PathBuf> {
    value.and_then(|value| {
        if value.as_bytes().len() > MAX_CONFIG_PATH_BYTES || value.to_str().is_none() {
            *was_truncated = true;
            None
        } else {
            Some(PathBuf::from(value))
        }
    })
}

fn normalized_config_path(value: Option<PathBuf>, was_truncated: &mut bool) -> Option<PathBuf> {
    value.and_then(|value| {
        if value.as_os_str().as_bytes().len() > MAX_CONFIG_PATH_BYTES || value.to_str().is_none() {
            *was_truncated = true;
            None
        } else {
            Some(value)
        }
    })
}

fn bounded_owned_warning_sources(
    settings_warning: Option<String>,
    discovery_warnings: Vec<String>,
) -> (Option<String>, Vec<String>, bool) {
    let mut remaining_bytes = MAX_SOURCE_WARNING_AGGREGATE_BYTES;
    let mut was_truncated = false;
    let settings_warning = settings_warning
        .into_iter()
        .take(MAX_SETTINGS_WARNINGS)
        .filter_map(|warning| {
            bounded_owned_warning(warning, &mut remaining_bytes, &mut was_truncated)
        })
        .next();

    let mut bounded_discovery = Vec::new();
    for (index, warning) in discovery_warnings
        .into_iter()
        .take(MAX_DISCOVERY_WARNINGS + 1)
        .enumerate()
    {
        if index == MAX_DISCOVERY_WARNINGS {
            was_truncated = true;
            break;
        }
        if let Some(warning) =
            bounded_owned_warning(warning, &mut remaining_bytes, &mut was_truncated)
        {
            bounded_discovery.push(warning);
        }
    }

    (settings_warning, bounded_discovery, was_truncated)
}

fn bounded_owned_warning(
    warning: String,
    remaining_bytes: &mut usize,
    was_truncated: &mut bool,
) -> Option<String> {
    if *remaining_bytes == 0 {
        *was_truncated = true;
        return None;
    }
    let limit = MAX_WARNING_BYTES.min(*remaining_bytes);
    let (warning, warning_was_truncated) = bounded_owned_text(warning, limit);
    *was_truncated |= warning_was_truncated;
    *remaining_bytes -= warning.len();
    Some(warning)
}

fn bounded_warning_sources<'a>(
    settings_warning: Option<&'a str>,
    discovery_warnings: impl IntoIterator<Item = &'a str>,
) -> (Option<String>, Vec<String>, bool) {
    let mut remaining_bytes = MAX_SOURCE_WARNING_AGGREGATE_BYTES;
    let mut was_truncated = false;
    let settings_warning = settings_warning
        .into_iter()
        .take(MAX_SETTINGS_WARNINGS)
        .filter_map(|warning| bounded_warning(warning, &mut remaining_bytes, &mut was_truncated))
        .next();

    let mut bounded_discovery = Vec::new();
    for (index, warning) in discovery_warnings
        .into_iter()
        .take(MAX_DISCOVERY_WARNINGS + 1)
        .enumerate()
    {
        if index == MAX_DISCOVERY_WARNINGS {
            was_truncated = true;
            break;
        }
        if let Some(warning) = bounded_warning(warning, &mut remaining_bytes, &mut was_truncated) {
            bounded_discovery.push(warning);
        }
    }

    (settings_warning, bounded_discovery, was_truncated)
}

fn bounded_warning(
    warning: &str,
    remaining_bytes: &mut usize,
    was_truncated: &mut bool,
) -> Option<String> {
    if *remaining_bytes == 0 {
        *was_truncated = true;
        return None;
    }
    let limit = MAX_WARNING_BYTES.min(*remaining_bytes);
    let (warning, warning_was_truncated) = bounded_text(warning, limit);
    *was_truncated |= warning_was_truncated;
    *remaining_bytes -= warning.len();
    Some(warning)
}

fn finalize_items(mut items: Vec<DiagnosticItem>, mut was_truncated: bool) -> Vec<DiagnosticItem> {
    let mut warning_bytes =
        MAX_RENDERED_WARNING_AGGREGATE_BYTES.saturating_sub(TRUNCATION_DETAIL.len());
    for item in &mut items {
        let (label, label_was_truncated) = bounded_text(&item.label, MAX_DIAGNOSTIC_LABEL_BYTES);
        item.label = label;
        was_truncated |= label_was_truncated;

        let detail_limit = if item.level == Level::Warning {
            MAX_DIAGNOSTIC_DETAIL_BYTES.min(warning_bytes)
        } else {
            MAX_DIAGNOSTIC_DETAIL_BYTES
        };
        let (detail, detail_was_truncated) = bounded_text(&item.detail, detail_limit);
        item.detail = detail;
        was_truncated |= detail_was_truncated;
        if item.level == Level::Warning {
            warning_bytes -= item.detail.len();
        }
    }

    if was_truncated {
        debug_assert!(TRUNCATION_LABEL.len() <= MAX_DIAGNOSTIC_LABEL_BYTES);
        debug_assert!(TRUNCATION_DETAIL.len() <= MAX_DIAGNOSTIC_DETAIL_BYTES);
        items.push(diagnostic(
            TRUNCATION_LABEL,
            TRUNCATION_DETAIL,
            Level::Warning,
        ));
    }
    items
}

fn bounded_text(value: &str, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        return (value.to_owned(), false);
    }
    let end = utf8_prefix_end(value, limit);
    (value[..end].to_owned(), true)
}

fn bounded_owned_text(mut value: String, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        return (value, false);
    }
    let end = utf8_prefix_end(&value, limit);
    value.truncate(end);
    (value, true)
}

fn utf8_prefix_end(value: &str, limit: usize) -> usize {
    let mut end = limit.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn path_exceeds_diagnostic_limit(path: &Path) -> bool {
    path.as_os_str().as_bytes().len() > MAX_CONFIG_PATH_BYTES
}

pub(crate) fn bounded_path_display(path: &Path, limit: usize) -> (String, bool) {
    if let Some(path) = path.to_str() {
        return bounded_text(path, limit);
    }

    let bytes = path.as_os_str().as_bytes();
    let prefix = &bytes[..bytes.len().min(limit)];
    let path = String::from_utf8_lossy(prefix);
    let (path, _text_was_truncated) = bounded_text(&path, limit);
    (path, true)
}

fn absolute_path(path: Option<&Path>) -> Option<&Path> {
    path.filter(|path| !path.as_os_str().is_empty() && path.is_absolute())
}

fn executable_availability(
    directories: &[PathBuf],
    executable_names: &[&str],
    executable_file: &mut impl FnMut(&Path) -> bool,
) -> Availability {
    for directory in directories {
        for executable_name in executable_names {
            if executable_file(&directory.join(executable_name)) {
                return Availability::Available;
            }
        }
    }
    Availability::Unavailable
}

fn bounded_path_value(value: OsString) -> (OsString, bool) {
    let bytes = value.as_bytes();
    if bytes.len() <= MAX_RAW_PATH_BYTES {
        return (value, false);
    }

    let complete_prefix_end = bytes[..MAX_RAW_PATH_BYTES]
        .iter()
        .rposition(|byte| *byte == b':')
        .unwrap_or(0);
    (
        OsString::from_vec(bytes[..complete_prefix_end].to_vec()),
        true,
    )
}

/// Follows symlinks through `metadata`, then accepts only a regular-file target
/// with at least one Unix execute bit. Directories and non-executable files fail closed.
pub(crate) fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0
    })
}

fn diagnostic(label: impl Into<String>, detail: impl Into<String>, level: Level) -> DiagnosticItem {
    DiagnosticItem {
        label: label.into(),
        detail: detail.into(),
        level,
    }
}
