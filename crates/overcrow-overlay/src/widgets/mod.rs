mod catalog;
mod chrome;
mod clock;
mod manager;
mod manual_stopwatch;
mod media;
mod notes;
mod performance;
mod registry;
mod session;
mod twitch_chat;
mod warframe_fissures;
mod warframe_invasions;
mod warframe_market;
mod warframe_sortie;
mod warframe_status;

pub(crate) use catalog::paint_gated_options;
pub(crate) use catalog::paint_profile_options;
pub(crate) use catalog::persist_profile_change;
pub use catalog::{
    CATALOG_ERROR_MAX_CHARS, CatalogAction, CatalogActionOutcome, CatalogCommit,
    CatalogFailureCategory, CatalogLayout, apply_catalog_action, catalog_visible, paint_catalog,
};
pub use chrome::{
    ACCENT, ControlIcon, PANEL_FILL, PANEL_STROKE_STRONG, TEXT_MUTED, install_theme, status_pill,
};
pub use clock::ClockPresentation;
pub use manager::{WidgetManager, placement_save_requested, widget_draggable, widget_visible};
pub use manual_stopwatch::{
    ManualStopwatchAction, ManualStopwatchClock, ManualStopwatchPresentation,
    format_manual_stopwatch_elapsed, manual_stopwatch_repaint_after, route_manual_stopwatch_action,
};
pub use media::{MediaControl, MediaPresentation};
pub use notes::{NotesWidgetState, notes_action_allowed};
pub use performance::PerformancePresentation;
pub use registry::{BUILTIN_WIDGETS, WidgetCategory, WidgetDescriptor, WidgetGlyph};
pub use session::{
    format_session_elapsed, session_draggable, session_repaint_after, session_visible,
};
pub use twitch_chat::{
    TwitchChatAction, TwitchChatResponse, TwitchWidgetState, passive_message_alpha,
    twitch_passive_repaint_after,
};
pub use warframe_fissures::{FissurePrefsAction, apply_fissure_prefs_action};
pub use warframe_invasions::{InvasionPrefsAction, apply_invasion_prefs_action};
pub use warframe_market::MarketUiAction;
pub use warframe_sortie::{SortiePrefsAction, apply_sortie_prefs_action};
pub use warframe_status::{StatusPrefsAction, apply_status_prefs_action};

pub(crate) use twitch_chat::paint_twitch_options;
pub(crate) use warframe_fissures::paint_fissure_options;
pub(crate) use warframe_invasions::paint_invasion_options;
pub(crate) use warframe_sortie::paint_sortie_options;
pub(crate) use warframe_status::paint_status_options;

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod presentation_tests;

#[cfg(test)]
#[path = "manual_stopwatch_tests.rs"]
mod manual_stopwatch_tests;

#[cfg(test)]
#[path = "warframe_tests.rs"]
mod warframe_tests;

#[cfg(test)]
#[path = "notes_tests.rs"]
mod notes_tests;

#[cfg(test)]
#[path = "twitch_chat_tests.rs"]
mod twitch_chat_tests;
