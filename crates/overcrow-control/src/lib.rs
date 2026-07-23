mod app;
mod commands;
mod compatibility;
mod diagnostics;
mod integration;
mod lifecycle;
mod model;
mod presentation;
mod settings_diagnostic;
mod steam;

pub use app::{
    APPLICATION_ID, APPLICATION_TITLE, AppTab, ControlController, ControlView, NoticeOperation,
    PickerFailure, PickerResult, SaveOutcome, ShortcutDisplay, UiNotice, UiNoticeLevel, WorkStart,
};
pub use commands::ControlCommandService;
pub use compatibility::{
    CompatibilityReason, CompatibilityReport, CompatibilityStatus, DesktopEnvironment,
    DisplaySession, EnvironmentIdentity, MAX_ENVIRONMENT_LABEL_BYTES,
};
pub use diagnostics::{
    Availability, DiagnosticInput, DiagnosticItem, DiagnosticReport, Level, PortalPickerInput,
    collect_foundation_diagnostics,
};
pub use integration::{IntegrationController, IntegrationSetup};
pub use lifecycle::{
    CommandRunner, CorePassiveClient, DbusCorePassiveClient, LifecycleController, LifecycleError,
    LifecycleManualIdentityValidator, LifecycleSettingsRepository, LifecycleSnapshot,
    LifecycleStatus, NativeLifecycleManualIdentityValidator, SystemCommandRunner,
};
pub use model::{ControlModel, NativePathValidator, PathValidator, SelectionError, SelectionStore};
pub use presentation::{
    CONTROL_LOG_SCHEMA_VERSION, CONTROL_SNAPSHOT_SCHEMA_VERSION, ControlCompatibility,
    ControlDiagnostic, ControlGame, ControlLifecycle, ControlLogSnapshot, ControlManualGame,
    ControlNotice, ControlOperationState, ControlSnapshot, DiagnosticLevelCode,
    MAX_CONTROL_GAME_NAME_BYTES, MAX_CONTROL_LOG_LINE_BYTES, MAX_CONTROL_LOG_LINES,
    MAX_CONTROL_LOG_RESPONSE_BYTES, MAX_CONTROL_SNAPSHOT_BYTES, NoticeLevelCode,
    NoticeOperationCode,
};
#[doc(hidden)]
pub use settings_diagnostic::{
    SETTINGS_DIAGNOSTIC_ARG, encode_settings_diagnostic, run_settings_diagnostic_request,
    settings_diagnostic_requested_from,
};
pub use steam::{
    DiscoveryReport, MAX_KEYVALUES_NESTING_DEPTH, MAX_LIBRARY_VDF_BYTES, MAX_MANIFEST_BYTES,
    MAX_MANIFESTS_INSPECTED, MAX_SECONDARY_LIBRARIES, MAX_WARNINGS, SteamGame,
    candidate_steam_roots, discover_from_roots,
};
#[cfg(test)]
#[path = "steam_tests.rs"]
mod steam_tests;

#[cfg(test)]
#[path = "model_tests.rs"]
mod model_tests;

#[cfg(test)]
#[path = "app_tests.rs"]
mod app_tests;

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod diagnostics_tests;

#[cfg(test)]
#[path = "integration_tests.rs"]
mod integration_tests;

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
#[path = "settings_diagnostic_tests.rs"]
mod settings_diagnostic_tests;

#[cfg(test)]
#[path = "compatibility_tests.rs"]
mod compatibility_tests;

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod presentation_tests;
