use eframe::egui::{Pos2, Rect, Vec2};
use overcrow_config::{WidgetId, WidgetProfile};
use overcrow_protocol::OverlayMode;

use super::{
    DiscordVoiceAction, TwitchChatAction, manual_stopwatch::ManualStopwatchAction,
    warframe_fissures::FissurePrefsAction, warframe_invasions::InvasionPrefsAction,
    warframe_market::MarketUiAction, warframe_sortie::SortiePrefsAction,
    warframe_status::StatusPrefsAction,
};
use crate::media::MediaAction;
use crate::notes::NotesCommand;

mod builtin;
mod controls;
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

pub struct NotesRenderOutcome {
    pub save_requested: bool,
    pub actions: Vec<NotesCommand>,
}

pub struct TwitchRenderOutcome {
    pub save_requested: bool,
    pub actions: Vec<TwitchChatAction>,
}

pub struct DiscordVoiceRenderOutcome {
    pub save_requested: bool,
    pub actions: Vec<DiscordVoiceAction>,
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
    measured_sizes: [[Vec2; WidgetId::COUNT]; 2],
    runtime_anchors: [Option<Pos2>; WidgetId::COUNT],
    visible_rects: [Option<Rect>; WidgetId::COUNT],
    visible_order: Vec<WidgetId>,
    pending_scales: [Option<f32>; WidgetId::COUNT],
    toolbar_open: Option<WidgetId>,
    toolbar_popup_id: Option<eframe::egui::Id>,
    catalog_open: bool,
    catalog_message: Option<String>,
    resize: Option<ResizeSession>,
    interaction_enabled: bool,
    viewport: Option<Rect>,
}

impl Default for WidgetManager {
    fn default() -> Self {
        Self {
            measured_sizes: [[Vec2::ZERO; WidgetId::COUNT]; 2],
            runtime_anchors: [None; WidgetId::COUNT],
            visible_rects: [None; WidgetId::COUNT],
            visible_order: Vec::with_capacity(WidgetId::ALL.len()),
            pending_scales: [None; WidgetId::COUNT],
            toolbar_open: None,
            toolbar_popup_id: None,
            catalog_open: false,
            catalog_message: None,
            resize: None,
            interaction_enabled: true,
            viewport: None,
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

    pub fn set_interaction_enabled(&mut self, enabled: bool) {
        self.interaction_enabled = enabled;
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
