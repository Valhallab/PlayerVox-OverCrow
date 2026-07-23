use std::time::Instant;

use eframe::egui;
use overcrow_config::{WidgetId, WidgetProfile};
use overcrow_protocol::CoreSnapshot;

use crate::{
    media::MediaSnapshot,
    session_clock::SessionClock,
    widgets::{
        clock::paint_clock,
        manual_stopwatch::{ManualStopwatchClock, paint_manual_stopwatch},
        media::paint_media,
        performance::paint_performance,
        session::{paint_session, session_draggable, session_visible},
    },
};

use super::{
    ManualStopwatchRenderOutcome, MediaRenderOutcome, WidgetManager, widget_draggable,
    widget_visible,
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
            self.screen_position(WidgetId::Session, viewport, margin, profile),
            clock.elapsed_at(now),
            profile.settings(WidgetId::Session).transparent_background,
            session_draggable(snapshot),
            margin,
        );
        self.finish_drag_only(
            WidgetId::Session,
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
            self.screen_position(WidgetId::Clock, viewport, margin, profile),
            profile.settings(WidgetId::Clock).transparent_background,
            widget_draggable(snapshot.overlay_mode, snapshot.active_game.is_some()),
            margin,
        );
        self.finish_drag_only(
            WidgetId::Clock,
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
            self.screen_position(WidgetId::Performance, viewport, margin, profile),
            snapshot.telemetry,
            profile
                .settings(WidgetId::Performance)
                .transparent_background,
            widget_draggable(snapshot.overlay_mode, snapshot.active_game.is_some()),
            margin,
        );
        self.finish_drag_only(
            WidgetId::Performance,
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
            self.screen_position(WidgetId::ManualStopwatch, viewport, margin, profile),
            clock.elapsed_at(now),
            clock.running(),
            snapshot.overlay_mode,
            profile
                .settings(WidgetId::ManualStopwatch)
                .transparent_background,
            widget_draggable(snapshot.overlay_mode, snapshot.active_game.is_some()),
            margin,
        );
        ManualStopwatchRenderOutcome {
            save_requested: self.finish_drag_only(
                WidgetId::ManualStopwatch,
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
            self.screen_position(WidgetId::Media, viewport, margin, profile),
            media_snapshot,
            core_snapshot.overlay_mode,
            profile.settings(WidgetId::Media).transparent_background,
            widget_draggable(
                core_snapshot.overlay_mode,
                core_snapshot.active_game.is_some(),
            ),
            margin,
        );
        MediaRenderOutcome {
            save_requested: self.finish_drag_only(
                WidgetId::Media,
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
}
