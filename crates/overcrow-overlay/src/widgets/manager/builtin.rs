use std::time::Instant;

use eframe::egui;
use overcrow_config::{TwitchPrefs, WidgetId, WidgetProfile};
use overcrow_protocol::CoreSnapshot;

use crate::{
    media::MediaSnapshot,
    session_clock::SessionClock,
    widgets::{
        clock::paint_clock,
        manual_stopwatch::{ManualStopwatchClock, paint_manual_stopwatch},
        media::paint_media,
        notes::{NotesWidgetState, paint_notes},
        performance::paint_performance,
        session::{paint_session, session_draggable, session_visible},
        twitch_chat::{TwitchWidgetState, paint_twitch_chat},
    },
};

use super::{
    ManualStopwatchRenderOutcome, MediaRenderOutcome, NotesRenderOutcome, TwitchRenderOutcome,
    WidgetManager, widget_draggable, widget_visible,
};

impl WidgetManager {
    pub fn render_session(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &CoreSnapshot,
        profile: &mut WidgetProfile,
        clock: &SessionClock,
        now: Instant,
        margin: f32,
    ) -> bool {
        if !session_visible(snapshot, profile) {
            return false;
        }

        let viewport = ui.max_rect();
        let response = paint_session(
            ui,
            self.screen_position(
                WidgetId::Session,
                snapshot.overlay_mode,
                viewport,
                margin,
                profile,
            ),
            clock.elapsed_at(now),
            profile.settings(WidgetId::Session).transparent_background,
            session_draggable(snapshot),
            margin,
        );
        self.request_repaint_if_size_changed(
            ui,
            WidgetId::Session,
            snapshot.overlay_mode,
            response.size,
        );
        self.finish_drag_only(
            WidgetId::Session,
            snapshot.overlay_mode,
            viewport,
            margin,
            profile,
            response.size,
            response.position,
            response.dragged,
            response.drag_stopped,
        )
    }

    pub fn render_clock(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &CoreSnapshot,
        profile: &mut WidgetProfile,
        margin: f32,
    ) -> bool {
        if !widget_visible(
            WidgetId::Clock,
            snapshot.overlay_mode,
            snapshot.active_game.is_some(),
            profile,
        ) {
            return false;
        }

        let viewport = ui.max_rect();
        let response = paint_clock(
            ui,
            self.screen_position(
                WidgetId::Clock,
                snapshot.overlay_mode,
                viewport,
                margin,
                profile,
            ),
            profile.settings(WidgetId::Clock).transparent_background,
            widget_draggable(snapshot.overlay_mode, snapshot.active_game.is_some()),
            margin,
        );
        self.request_repaint_if_size_changed(
            ui,
            WidgetId::Clock,
            snapshot.overlay_mode,
            response.size,
        );
        self.finish_drag_only(
            WidgetId::Clock,
            snapshot.overlay_mode,
            viewport,
            margin,
            profile,
            response.size,
            response.position,
            response.dragged,
            response.drag_stopped,
        )
    }

    pub fn render_performance(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &CoreSnapshot,
        profile: &mut WidgetProfile,
        margin: f32,
    ) -> bool {
        if !widget_visible(
            WidgetId::Performance,
            snapshot.overlay_mode,
            snapshot.active_game.is_some(),
            profile,
        ) {
            return false;
        }

        let viewport = ui.max_rect();
        let response = paint_performance(
            ui,
            self.screen_position(
                WidgetId::Performance,
                snapshot.overlay_mode,
                viewport,
                margin,
                profile,
            ),
            snapshot.telemetry,
            profile
                .settings(WidgetId::Performance)
                .transparent_background,
            widget_draggable(snapshot.overlay_mode, snapshot.active_game.is_some()),
            margin,
        );
        self.request_repaint_if_size_changed(
            ui,
            WidgetId::Performance,
            snapshot.overlay_mode,
            response.size,
        );
        self.finish_drag_only(
            WidgetId::Performance,
            snapshot.overlay_mode,
            viewport,
            margin,
            profile,
            response.size,
            response.position,
            response.dragged,
            response.drag_stopped,
        )
    }

    pub fn render_manual_stopwatch(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &CoreSnapshot,
        profile: &mut WidgetProfile,
        clock: &ManualStopwatchClock,
        now: Instant,
        margin: f32,
    ) -> ManualStopwatchRenderOutcome {
        if !widget_visible(
            WidgetId::ManualStopwatch,
            snapshot.overlay_mode,
            snapshot.active_game.is_some(),
            profile,
        ) {
            return ManualStopwatchRenderOutcome {
                save_requested: false,
                action: None,
            };
        }

        let viewport = ui.max_rect();
        let response = paint_manual_stopwatch(
            ui,
            self.screen_position(
                WidgetId::ManualStopwatch,
                snapshot.overlay_mode,
                viewport,
                margin,
                profile,
            ),
            clock.elapsed_at(now),
            clock.running(),
            snapshot.overlay_mode,
            profile
                .settings(WidgetId::ManualStopwatch)
                .transparent_background,
            widget_draggable(snapshot.overlay_mode, snapshot.active_game.is_some()),
            margin,
        );
        self.request_repaint_if_size_changed(
            ui,
            WidgetId::ManualStopwatch,
            snapshot.overlay_mode,
            response.size,
        );
        ManualStopwatchRenderOutcome {
            save_requested: self.finish_drag_only(
                WidgetId::ManualStopwatch,
                snapshot.overlay_mode,
                viewport,
                margin,
                profile,
                response.size,
                response.position,
                response.dragged,
                response.drag_stopped,
            ),
            action: response.action,
        }
    }

    pub fn render_media(
        &mut self,
        ui: &mut egui::Ui,
        core_snapshot: &CoreSnapshot,
        media_snapshot: &MediaSnapshot,
        profile: &mut WidgetProfile,
        margin: f32,
    ) -> MediaRenderOutcome {
        if !widget_visible(
            WidgetId::Media,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            profile,
        ) {
            return MediaRenderOutcome {
                save_requested: false,
                action: None,
            };
        }

        let viewport = ui.max_rect();
        let response = paint_media(
            ui,
            self.screen_position(
                WidgetId::Media,
                core_snapshot.overlay_mode,
                viewport,
                margin,
                profile,
            ),
            media_snapshot,
            core_snapshot.overlay_mode,
            profile.settings(WidgetId::Media).transparent_background,
            widget_draggable(
                core_snapshot.overlay_mode,
                core_snapshot.active_game.is_some(),
            ),
            margin,
        );
        self.request_repaint_if_size_changed(
            ui,
            WidgetId::Media,
            core_snapshot.overlay_mode,
            response.size,
        );
        MediaRenderOutcome {
            save_requested: self.finish_drag_only(
                WidgetId::Media,
                core_snapshot.overlay_mode,
                viewport,
                margin,
                profile,
                response.size,
                response.position,
                response.dragged,
                response.drag_stopped,
            ),
            action: response.action,
        }
    }

    pub fn render_notes(
        &mut self,
        ui: &mut egui::Ui,
        core_snapshot: &CoreSnapshot,
        state: &mut NotesWidgetState,
        profile: &mut WidgetProfile,
        margin: f32,
    ) -> NotesRenderOutcome {
        let id = WidgetId::Notes;
        if !widget_visible(
            id,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            profile,
        ) {
            return NotesRenderOutcome {
                save_requested: false,
                actions: Vec::new(),
            };
        }

        let viewport = ui.max_rect();
        let position =
            self.screen_position(id, core_snapshot.overlay_mode, viewport, margin, profile);
        let panel_size = self.panel_size(id, core_snapshot.overlay_mode, profile);
        let can_move = self.can_move_panel(
            ui,
            id,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            position,
            panel_size,
        );
        let response = paint_notes(
            ui,
            position,
            panel_size,
            state,
            profile.notes_display,
            profile.settings(id).scale,
            core_snapshot.overlay_mode,
            profile.settings(id).transparent_background,
            can_move,
            margin,
        );
        self.request_repaint_if_size_changed(ui, id, core_snapshot.overlay_mode, response.size);
        NotesRenderOutcome {
            save_requested: self.finish_resizable_panel(
                id,
                core_snapshot.overlay_mode,
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
    pub fn render_twitch(
        &mut self,
        ui: &mut egui::Ui,
        core_snapshot: &CoreSnapshot,
        state: &mut TwitchWidgetState,
        twitch_prefs: &TwitchPrefs,
        profile: &mut WidgetProfile,
        now: Instant,
        margin: f32,
    ) -> TwitchRenderOutcome {
        let id = WidgetId::TwitchChat;
        if !widget_visible(
            id,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            profile,
        ) {
            return TwitchRenderOutcome {
                save_requested: false,
                actions: Vec::new(),
            };
        }

        let viewport = ui.max_rect();
        let position =
            self.screen_position(id, core_snapshot.overlay_mode, viewport, margin, profile);
        let panel_size = self.panel_size(id, core_snapshot.overlay_mode, profile);
        let can_move = self.can_move_panel(
            ui,
            id,
            core_snapshot.overlay_mode,
            core_snapshot.active_game.is_some(),
            position,
            panel_size,
        );
        let response = paint_twitch_chat(
            ui,
            position,
            panel_size,
            state,
            twitch_prefs,
            profile.settings(id).scale,
            core_snapshot.overlay_mode,
            profile.settings(id).transparent_background,
            can_move,
            margin,
            now,
        );
        self.request_repaint_if_size_changed(ui, id, core_snapshot.overlay_mode, response.size);
        TwitchRenderOutcome {
            save_requested: self.finish_resizable_panel(
                id,
                core_snapshot.overlay_mode,
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
