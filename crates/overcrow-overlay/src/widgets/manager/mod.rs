use eframe::egui::{Pos2, Vec2};
use overcrow_config::{WidgetId, WidgetProfile};
use overcrow_protocol::OverlayMode;

use crate::media::MediaAction;

use super::{
    manual_stopwatch::ManualStopwatchAction, warframe_fissures::FissurePrefsAction,
    warframe_invasions::InvasionPrefsAction, warframe_market::MarketUiAction,
    warframe_sortie::SortiePrefsAction, warframe_status::StatusPrefsAction,
};

mod builtin;
mod layout;
mod warframe;

pub use layout::placement_save_requested;

pub struct ManualStopwatchRenderOutcome {
    pub save_requested: bool,
    pub action: Option<ManualStopwatchAction>,
}

pub struct MediaRenderOutcome {
    pub save_requested: bool,
    pub action: Option<MediaAction>,
}

pub struct WarframeStatusRenderOutcome {
    pub save_requested: bool,
    pub actions: Vec<StatusPrefsAction>,
}

pub struct WarframeFissuresRenderOutcome {
    pub save_requested: bool,
    pub actions: Vec<FissurePrefsAction>,
}

pub struct WarframeMarketRenderOutcome {
    pub save_requested: bool,
    pub actions: Vec<MarketUiAction>,
}

pub struct WarframeSortieRenderOutcome {
    pub save_requested: bool,
    pub actions: Vec<SortiePrefsAction>,
}

pub struct WarframeInvasionsRenderOutcome {
    pub save_requested: bool,
    pub actions: Vec<InvasionPrefsAction>,
}

/// Active bottom-right resize: absolute top-left is frozen for the whole gesture.
#[derive(Clone, Copy, Debug)]
struct ResizeSession {
    id: WidgetId,
    /// Fixed screen-space top-left for the entire drag.
    anchor: Pos2,
    /// Current panel size during the drag.
    size: Vec2,
    /// True if size actually changed at least once (not a pure min-size tug).
    size_changed: bool,
}

#[derive(Debug)]
pub struct WidgetManager {
    measured_sizes: [Vec2; 11],
    catalog_open: bool,
    catalog_message: Option<String>,
    resize: Option<ResizeSession>,
}

impl Default for WidgetManager {
    fn default() -> Self {
        Self {
            measured_sizes: [Vec2::ZERO; 11],
            catalog_open: false,
            catalog_message: None,
            resize: None,
        }
    }
}

impl WidgetManager {
    pub fn catalog_open(&self) -> bool {
        self.catalog_open
    }

    pub fn set_catalog_open(&mut self, open: bool) {
        self.catalog_open = open;
    }

    pub fn catalog_message(&self) -> Option<&str> {
        self.catalog_message.as_deref()
    }

    pub fn set_catalog_message(&mut self, message: Option<String>) {
        self.catalog_message = message;
    }
}

pub fn widget_visible(
    id: WidgetId,
    mode: OverlayMode,
    active_game: bool,
    profile: &WidgetProfile,
) -> bool {
    let settings = profile.settings(id);
    active_game
        && settings.enabled
        && (mode == OverlayMode::Interactive || settings.show_in_passive)
}

pub fn widget_draggable(mode: OverlayMode, active_game: bool) -> bool {
    active_game && mode == OverlayMode::Interactive
}

#[cfg(test)]
mod tests;
