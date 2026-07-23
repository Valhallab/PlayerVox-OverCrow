mod allowlist;
mod model;
mod store;
mod warframe_prefs;
mod widget_store;
mod widgets;

pub use allowlist::{GameAllowlist, ProcessIdentity};
pub use model::{
    LifecycleSettings, ManualGame, SETTINGS_SCHEMA_VERSION, SettingsError, ShortcutSettings,
};
pub use store::{
    CommittedSettingsSaveError, SETTINGS_DIAGNOSTIC_MAX_BYTES, SETTINGS_MAX_BYTES,
    SettingsDiagnostic, SettingsLoad, SettingsStore, settings_save_was_committed,
};
pub use warframe_prefs::{
    FissureEra, FissureSource, StatusRow, WARFRAME_ACTIVITY_DONE_ENTRY_MAX_CHARS,
    WARFRAME_INVASION_WATCHLIST_ENTRY_MAX_CHARS, WARFRAME_INVASION_WATCHLIST_MAX,
    WARFRAME_MARKET_QUERY_MAX_CHARS, WARFRAME_PREFS_MAX_BYTES, WARFRAME_PREFS_SCHEMA_VERSION,
    WARFRAME_STEAM_APP_ID, WarframePrefs, WarframePrefsError, WarframePrefsLoad,
    WarframePrefsStore, path_tail,
};
pub use widget_store::{WidgetSettingsLoad, WidgetSettingsStore};
pub use widgets::{
    WIDGET_PANEL_MAX, WIDGET_PANEL_MIN, WIDGET_PANEL_MIN_HEIGHT, WIDGET_SCALE_MAX,
    WIDGET_SCALE_MIN, WIDGET_SCHEMA_VERSION, WidgetId, WidgetPosition, WidgetProfile,
    WidgetProfileError, WidgetSettings,
};

#[cfg(test)]
#[path = "model_tests.rs"]
mod model_tests;

#[cfg(test)]
#[path = "allowlist_tests.rs"]
mod allowlist_tests;
