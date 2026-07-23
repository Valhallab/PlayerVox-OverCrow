mod catalog;
mod chrome;
mod clock;
mod manager;
mod manual_stopwatch;
mod media;
mod performance;
mod registry;
mod session;
mod warframe_fissures;
mod warframe_invasions;
mod warframe_market;
mod warframe_sortie;
mod warframe_status;

pub use catalog::{
    CATALOG_ERROR_MAX_CHARS, CatalogAction, CatalogActionOutcome, CatalogCommit,
    CatalogFailureCategory, apply_catalog_action, catalog_visible, paint_catalog,
};
pub use clock::ClockPresentation;
pub use manager::{WidgetManager, placement_save_requested, widget_draggable, widget_visible};
pub use manual_stopwatch::{
    ManualStopwatchAction, ManualStopwatchClock, ManualStopwatchPresentation,
    format_manual_stopwatch_elapsed, manual_stopwatch_repaint_after, route_manual_stopwatch_action,
};
pub use media::{MediaControl, MediaPresentation};
pub use performance::PerformancePresentation;
pub use registry::{BUILTIN_WIDGETS, WidgetDescriptor};
pub use session::{
    format_session_elapsed, session_draggable, session_repaint_after, session_visible,
};
pub use warframe_fissures::{FissurePrefsAction, apply_fissure_prefs_action};
pub use warframe_invasions::{InvasionPrefsAction, apply_invasion_prefs_action};
pub use warframe_market::MarketUiAction;
pub use warframe_sortie::{SortiePrefsAction, apply_sortie_prefs_action};
pub use warframe_status::{StatusPrefsAction, apply_status_prefs_action};

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod presentation_tests;

#[cfg(test)]
#[path = "manual_stopwatch_tests.rs"]
mod manual_stopwatch_tests;

#[cfg(test)]
#[path = "warframe_tests.rs"]
mod warframe_tests;
