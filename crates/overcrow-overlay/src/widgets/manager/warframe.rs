use eframe::egui;
use overcrow_config::{WarframePrefs, WidgetId, WidgetProfile};
use overcrow_protocol::CoreSnapshot;

use crate::{
    warframe::{MarketSnapshot, WarframeDerivedCache, WorldstateSnapshot, warframe_widget_visible},
    widgets::{
        warframe_fissures::paint_warframe_fissures, warframe_invasions::paint_warframe_invasions,
        warframe_market::paint_warframe_market, warframe_sortie::paint_warframe_sortie,
        warframe_status::paint_warframe_status,
    },
};

use super::{
    WarframeFissuresRenderOutcome, WarframeInvasionsRenderOutcome, WarframeMarketRenderOutcome,
    WarframeSortieRenderOutcome, WarframeStatusRenderOutcome, WidgetManager, widget_visible,
};

impl WidgetManager {
    #[allow(clippy::too_many_arguments)]
    pub fn render_warframe_status(
        &mut self,
        ui: &mut egui::Ui,
        core_snapshot: &CoreSnapshot,
        worldstate: &WorldstateSnapshot,
        prefs: &WarframePrefs,
        profile: &mut WidgetProfile,
        now_secs: u64,
        margin: f32,
    ) -> WarframeStatusRenderOutcome {
        let base = widget_visible(
            WidgetId::WarframeStatus,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            profile,
        );
        if !warframe_widget_visible(WidgetId::WarframeStatus, core_snapshot, profile, base) {
            return WarframeStatusRenderOutcome {
                save_requested: false,
                actions: Vec::new(),
            };
        }

        let viewport = ui.max_rect();
        let id = WidgetId::WarframeStatus;
        let pos = self.screen_position(id, viewport, margin, profile);
        let panel_size = self.panel_size_for(id, profile);
        let can_move = self.can_move_panel(
            ui,
            id,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            pos,
            panel_size,
        );
        let response = paint_warframe_status(
            ui,
            pos,
            panel_size,
            worldstate,
            prefs,
            profile.settings(id).scale,
            core_snapshot.overlay_mode,
            now_secs,
            profile.settings(id).transparent_background,
            can_move,
            margin,
        );
        WarframeStatusRenderOutcome {
            save_requested: self.finish_warframe_panel(
                id,
                viewport,
                margin,
                profile,
                response.size,
                response.position,
                response.dragged,
                response.drag_stopped,
                response.resize,
            ),
            actions: response.actions,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_warframe_fissures(
        &mut self,
        ui: &mut egui::Ui,
        core_snapshot: &CoreSnapshot,
        worldstate: &WorldstateSnapshot,
        fissure_indices: &[usize],
        prefs: &WarframePrefs,
        profile: &mut WidgetProfile,
        now_secs: u64,
        margin: f32,
    ) -> WarframeFissuresRenderOutcome {
        let base = widget_visible(
            WidgetId::WarframeFissures,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            profile,
        );
        if !warframe_widget_visible(WidgetId::WarframeFissures, core_snapshot, profile, base) {
            return WarframeFissuresRenderOutcome {
                save_requested: false,
                actions: Vec::new(),
            };
        }

        let viewport = ui.max_rect();
        let id = WidgetId::WarframeFissures;
        let pos = self.screen_position(id, viewport, margin, profile);
        let panel_size = self.panel_size_for(id, profile);
        let can_move = self.can_move_panel(
            ui,
            id,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            pos,
            panel_size,
        );
        let response = paint_warframe_fissures(
            ui,
            pos,
            panel_size,
            &worldstate.fissures,
            fissure_indices,
            prefs,
            profile.settings(id).scale,
            core_snapshot.overlay_mode,
            now_secs,
            profile.settings(id).transparent_background,
            can_move,
            margin,
        );
        WarframeFissuresRenderOutcome {
            save_requested: self.finish_warframe_panel(
                id,
                viewport,
                margin,
                profile,
                response.size,
                response.position,
                response.dragged,
                response.drag_stopped,
                response.resize,
            ),
            actions: response.actions,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_warframe_market(
        &mut self,
        ui: &mut egui::Ui,
        core_snapshot: &CoreSnapshot,
        market: &MarketSnapshot,
        draft_query: &mut String,
        copy_feedback: Option<&str>,
        profile: &mut WidgetProfile,
        margin: f32,
    ) -> WarframeMarketRenderOutcome {
        let base = widget_visible(
            WidgetId::WarframeMarket,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            profile,
        );
        if !warframe_widget_visible(WidgetId::WarframeMarket, core_snapshot, profile, base) {
            return WarframeMarketRenderOutcome {
                save_requested: false,
                actions: Vec::new(),
            };
        }

        let viewport = ui.max_rect();
        let id = WidgetId::WarframeMarket;
        let pos = self.screen_position(id, viewport, margin, profile);
        let panel_size = self.panel_size_for(id, profile);
        let can_move = self.can_move_panel(
            ui,
            id,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            pos,
            panel_size,
        );
        let response = paint_warframe_market(
            ui,
            pos,
            panel_size,
            market,
            draft_query,
            copy_feedback,
            profile.settings(id).scale,
            core_snapshot.overlay_mode,
            profile.settings(id).transparent_background,
            can_move,
            margin,
        );
        WarframeMarketRenderOutcome {
            save_requested: self.finish_warframe_panel(
                id,
                viewport,
                margin,
                profile,
                response.size,
                response.position,
                response.dragged,
                response.drag_stopped,
                response.resize,
            ),
            actions: response.actions,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_warframe_sortie(
        &mut self,
        ui: &mut egui::Ui,
        core_snapshot: &CoreSnapshot,
        worldstate: &WorldstateSnapshot,
        prefs: &WarframePrefs,
        profile: &mut WidgetProfile,
        now_secs: u64,
        margin: f32,
    ) -> WarframeSortieRenderOutcome {
        let base = widget_visible(
            WidgetId::WarframeSortie,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            profile,
        );
        if !warframe_widget_visible(WidgetId::WarframeSortie, core_snapshot, profile, base) {
            return WarframeSortieRenderOutcome {
                save_requested: false,
                actions: Vec::new(),
            };
        }

        let viewport = ui.max_rect();
        let id = WidgetId::WarframeSortie;
        let pos = self.screen_position(id, viewport, margin, profile);
        let panel_size = self.panel_size_for(id, profile);
        let can_move = self.can_move_panel(
            ui,
            id,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            pos,
            panel_size,
        );
        let response = paint_warframe_sortie(
            ui,
            pos,
            panel_size,
            worldstate,
            prefs,
            profile.settings(id).scale,
            core_snapshot.overlay_mode,
            now_secs,
            profile.settings(id).transparent_background,
            can_move,
            margin,
        );
        WarframeSortieRenderOutcome {
            save_requested: self.finish_warframe_panel(
                id,
                viewport,
                margin,
                profile,
                response.size,
                response.position,
                response.dragged,
                response.drag_stopped,
                response.resize,
            ),
            actions: response.actions,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_warframe_invasions(
        &mut self,
        ui: &mut egui::Ui,
        core_snapshot: &CoreSnapshot,
        worldstate: &WorldstateSnapshot,
        invasion_indices: &[usize],
        reward_catalog: &[(String, String)],
        derived_cache: &mut WarframeDerivedCache,
        worldstate_revision: u64,
        prefs_revision: u64,
        prefs: &WarframePrefs,
        profile: &mut WidgetProfile,
        margin: f32,
    ) -> WarframeInvasionsRenderOutcome {
        let base = widget_visible(
            WidgetId::WarframeInvasions,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            profile,
        );
        if !warframe_widget_visible(WidgetId::WarframeInvasions, core_snapshot, profile, base) {
            return WarframeInvasionsRenderOutcome {
                save_requested: false,
                actions: Vec::new(),
            };
        }

        let viewport = ui.max_rect();
        let id = WidgetId::WarframeInvasions;
        let pos = self.screen_position(id, viewport, margin, profile);
        let panel_size = self.panel_size_for(id, profile);
        let can_move = self.can_move_panel(
            ui,
            id,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            pos,
            panel_size,
        );
        let response = paint_warframe_invasions(
            ui,
            pos,
            panel_size,
            &worldstate.invasions,
            invasion_indices,
            reward_catalog,
            derived_cache,
            worldstate_revision,
            prefs_revision,
            prefs,
            profile.settings(id).scale,
            core_snapshot.overlay_mode,
            profile.settings(id).transparent_background,
            can_move,
            margin,
        );
        WarframeInvasionsRenderOutcome {
            save_requested: self.finish_warframe_panel(
                id,
                viewport,
                margin,
                profile,
                response.size,
                response.position,
                response.dragged,
                response.drag_stopped,
                response.resize,
            ),
            actions: response.actions,
        }
    }
}
