use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    CompatibilityReason, CompatibilityReport, CompatibilityStatus, DesktopEnvironment,
    DisplaySession, Level, LifecycleStatus, NoticeOperation, UiNoticeLevel,
    compatibility::MAX_ENVIRONMENT_LABEL_BYTES, model::is_stable_manual_game_id,
};

pub const CONTROL_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const MAX_CONTROL_SNAPSHOT_BYTES: usize = 512 * 1024;
pub const CONTROL_LOG_SCHEMA_VERSION: u32 = 1;
pub const MAX_CONTROL_LOG_LINES: usize = 500;
pub const MAX_CONTROL_LOG_LINE_BYTES: usize = overcrow_logging::MAX_LINE_BYTES - 1;
pub const MAX_CONTROL_LOG_RESPONSE_BYTES: usize = 256 * 1024;
pub const MAX_CONTROL_GAME_NAME_BYTES: usize = 256;
pub(crate) const MAX_CONTROL_MESSAGE_BYTES: usize = 512;
pub(crate) const MAX_CONTROL_DIAGNOSTIC_LABEL_BYTES: usize = 64;
pub(crate) const MAX_CONTROL_PATH_BYTES: usize = 512;
pub(crate) const MAX_CONTROL_SHORTCUT_BYTES: usize = 64;
pub(crate) const MAX_CONTROL_ID_BYTES: usize = 128;
pub(crate) const MAX_CONTROL_GAMES: usize = 256;
pub(crate) const MAX_CONTROL_MANUAL_GAMES: usize = 128;
pub(crate) const MAX_CONTROL_NOTICES: usize = 4;
pub(crate) const MAX_CONTROL_DIAGNOSTICS: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlSnapshot {
    pub schema_version: u32,
    pub compatibility: ControlCompatibility,
    pub lifecycle: ControlLifecycle,
    pub master_switch_enabled: bool,
    pub master_switch_checked: bool,
    pub selection_editing_enabled: bool,
    pub shortcut: String,
    pub operations: ControlOperationState,
    pub games: Vec<ControlGame>,
    pub manual_games: Vec<ControlManualGame>,
    pub notices: Vec<ControlNotice>,
    pub diagnostics: Vec<ControlDiagnostic>,
}

impl ControlSnapshot {
    /// Verifies the allocation and identity bounds expected on the D-Bus wire.
    pub fn has_valid_wire_bounds(&self) -> bool {
        if self.compatibility.operating_system.len() > MAX_ENVIRONMENT_LABEL_BYTES
            || self.shortcut.len() > MAX_CONTROL_SHORTCUT_BYTES
            || self.games.len() > MAX_CONTROL_GAMES
            || self.manual_games.len() > MAX_CONTROL_MANUAL_GAMES
            || self.notices.len() > MAX_CONTROL_NOTICES
            || self.diagnostics.len() > MAX_CONTROL_DIAGNOSTICS
        {
            return false;
        }

        let mut app_ids = BTreeSet::new();
        if !self.games.iter().all(|game| {
            game.app_id != 0
                && app_ids.insert(game.app_id)
                && game.name.len() <= MAX_CONTROL_GAME_NAME_BYTES
        }) {
            return false;
        }

        let mut manual_ids = BTreeSet::new();
        if !self.manual_games.iter().all(|game| {
            is_stable_manual_game_id(&game.id)
                && manual_ids.insert(game.id.as_str())
                && game.name.len() <= MAX_CONTROL_GAME_NAME_BYTES
                && game.executable.len() <= MAX_CONTROL_PATH_BYTES
        }) {
            return false;
        }

        self.notices
            .iter()
            .all(|notice| notice.message.len() <= MAX_CONTROL_MESSAGE_BYTES)
            && self.diagnostics.iter().all(|item| {
                item.label.len() <= MAX_CONTROL_DIAGNOSTIC_LABEL_BYTES
                    && item.detail.len() <= MAX_CONTROL_MESSAGE_BYTES
            })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlLogSnapshot {
    pub schema_version: u32,
    pub lines: Vec<String>,
    pub truncated: bool,
}

impl ControlLogSnapshot {
    pub fn from_recent_lines(mut lines: Vec<String>) -> Option<Self> {
        if !lines.iter().all(|line| valid_control_log_line(line)) {
            return None;
        }

        let mut truncated = lines.len() > MAX_CONTROL_LOG_LINES;
        let excess = lines.len().saturating_sub(MAX_CONTROL_LOG_LINES);
        if excess != 0 {
            lines.drain(..excess);
        }

        loop {
            let snapshot = Self {
                schema_version: CONTROL_LOG_SCHEMA_VERSION,
                lines,
                truncated,
            };
            let serialized = serde_json::to_vec(&snapshot).ok()?;
            if serialized.len() <= MAX_CONTROL_LOG_RESPONSE_BYTES {
                return Some(snapshot);
            }
            if snapshot.lines.is_empty() {
                return None;
            }
            lines = snapshot.lines;
            lines.remove(0);
            truncated = true;
        }
    }

    pub fn has_valid_wire_bounds(&self) -> bool {
        self.schema_version == CONTROL_LOG_SCHEMA_VERSION
            && self.lines.len() <= MAX_CONTROL_LOG_LINES
            && self.lines.iter().all(|line| valid_control_log_line(line))
            && serde_json::to_vec(self)
                .is_ok_and(|serialized| serialized.len() <= MAX_CONTROL_LOG_RESPONSE_BYTES)
    }
}

fn valid_control_log_line(line: &str) -> bool {
    !line.is_empty()
        && line.len() <= MAX_CONTROL_LOG_LINE_BYTES
        && !line.bytes().any(|byte| byte.is_ascii_control())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCompatibility {
    pub operating_system: String,
    pub session: DisplaySession,
    pub desktop: DesktopEnvironment,
    pub status: CompatibilityStatus,
    pub reason: CompatibilityReason,
    pub activation_allowed: bool,
}

impl From<&CompatibilityReport> for ControlCompatibility {
    fn from(report: &CompatibilityReport) -> Self {
        Self {
            operating_system: report.operating_system.clone(),
            session: report.session,
            desktop: report.desktop,
            status: report.status,
            reason: report.reason,
            activation_allowed: report.activation_allowed,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlLifecycle {
    Disabled,
    Enabled,
    Warning,
    Enabling,
    Disabling,
}

impl ControlLifecycle {
    pub(crate) fn from_status_and_label(status: LifecycleStatus, label: &str) -> Self {
        match label {
            "Enabling…" => Self::Enabling,
            "Disabling…" => Self::Disabling,
            _ => match status {
                LifecycleStatus::Disabled => Self::Disabled,
                LifecycleStatus::Enabled => Self::Enabled,
                LifecycleStatus::Warning => Self::Warning,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlOperationState {
    pub refresh: bool,
    pub picker: bool,
    pub lifecycle: bool,
}

impl ControlOperationState {
    pub const fn any_in_flight(self) -> bool {
        self.refresh || self.picker || self.lifecycle
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlGame {
    pub app_id: u32,
    pub name: String,
    pub selected: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlManualGame {
    pub id: String,
    pub name: String,
    pub executable: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeOperationCode {
    SelectionSave,
    Refresh,
    Picker,
    Lifecycle,
}

impl From<NoticeOperation> for NoticeOperationCode {
    fn from(operation: NoticeOperation) -> Self {
        match operation {
            NoticeOperation::SelectionSave => Self::SelectionSave,
            NoticeOperation::Refresh => Self::Refresh,
            NoticeOperation::Picker => Self::Picker,
            NoticeOperation::Lifecycle => Self::Lifecycle,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevelCode {
    Warning,
    Error,
}

impl From<UiNoticeLevel> for NoticeLevelCode {
    fn from(level: UiNoticeLevel) -> Self {
        match level {
            UiNoticeLevel::Warning => Self::Warning,
            UiNoticeLevel::Error => Self::Error,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlNotice {
    pub operation: NoticeOperationCode,
    pub level: NoticeLevelCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevelCode {
    Ok,
    Info,
    Warning,
    Error,
}

impl From<Level> for DiagnosticLevelCode {
    fn from(level: Level) -> Self {
        match level {
            Level::Ok => Self::Ok,
            Level::Info => Self::Info,
            Level::Warning => Self::Warning,
            Level::Error => Self::Error,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlDiagnostic {
    pub label: String,
    pub detail: String,
    pub level: DiagnosticLevelCode,
}

pub(crate) fn bounded_control_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
