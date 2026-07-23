use eframe::egui::{self, Pos2, Rect, Vec2};
use overcrow_config::{
    WIDGET_PANEL_MAX, WIDGET_PANEL_MIN, WIDGET_PANEL_MIN_HEIGHT, WidgetId, WidgetProfile,
};
use overcrow_protocol::OverlayMode;

use crate::placement;

use super::{ResizeSession, WidgetManager, widget_draggable};

impl WidgetManager {
    pub fn measured_size(&self, id: WidgetId) -> Vec2 {
        self.measured_sizes[widget_index(id)]
    }

    pub fn set_measured_size(&mut self, id: WidgetId, size: Vec2) {
        self.measured_sizes[widget_index(id)] = size;
    }

    pub fn sync_interaction_state(
        &mut self,
        mode: OverlayMode,
        active_game: bool,
        pointer_down: bool,
    ) {
        if mode != OverlayMode::Interactive || !active_game || !pointer_down {
            self.resize = None;
        }
    }

    pub fn screen_position(
        &self,
        id: WidgetId,
        viewport: Rect,
        margin: f32,
        profile: &WidgetProfile,
    ) -> Pos2 {
        // During resize, freeze the absolute top-left.
        if let Some(session) = self.resize
            && session.id == id
        {
            return session.anchor;
        }
        // Prefer last measured size when available (tests + post-drag); otherwise
        // fall back to configured panel size for Warframe widgets.
        let measured = self.measured_size(id);
        let size = if measured.x > 1.0 && measured.y > 1.0 {
            measured
        } else {
            let (w, h) = profile.settings(id).effective_panel_size(id);
            eframe::egui::vec2(w.max(1.0), h.max(1.0))
        };
        placement::screen_position(viewport, size, margin, profile.settings(id).position)
    }

    pub(super) fn panel_size_for(&self, id: WidgetId, profile: &WidgetProfile) -> Vec2 {
        if let Some(session) = self.resize
            && session.id == id
        {
            return session.size;
        }
        let (w, h) = profile.settings(id).effective_panel_size(id);
        eframe::egui::vec2(w.max(1.0), h.max(1.0))
    }

    pub(super) fn can_move_panel(
        &self,
        ui: &egui::Ui,
        id: WidgetId,
        mode: OverlayMode,
        active_game: bool,
        top_left: Pos2,
        panel_size: Vec2,
    ) -> bool {
        if !widget_draggable(mode, active_game) {
            return false;
        }
        if self.resize.is_some_and(|s| s.id == id) {
            return false;
        }
        // Avoid treating grip drags as Area moves (even on the first frame).
        !pointer_near_resize_grip(ui, top_left, panel_size)
    }

    /// Apply grip drag: absolute top-left stays fixed; only size changes.
    /// Pure min-size tugs do not rewrite position on release.
    #[allow(clippy::too_many_arguments)]
    fn apply_resize_grip(
        &mut self,
        id: WidgetId,
        viewport: Rect,
        margin: f32,
        profile: &mut WidgetProfile,
        rendered_size: Vec2,
        visible_top_left: Pos2,
        grip: crate::widgets::chrome::ResizeGripOutcome,
    ) -> bool {
        if grip.dragging {
            if self.resize.is_none() {
                let size = self.panel_size_for(id, profile);
                self.resize = Some(ResizeSession {
                    id,
                    anchor: visible_top_left,
                    size,
                    size_changed: false,
                });
            }
            if let Some(session) = self.resize.as_mut().filter(|s| s.id == id) {
                let min_w = panel_min_width(id);
                let next = crate::widgets::chrome::clamp_panel_size_min(
                    session.size + clamp_delta_at_limits(session.size, grip.drag_delta, min_w),
                    min_w,
                );
                if size_meaningfully_changed(session.size, next) {
                    session.size = next;
                    session.size_changed = true;
                    let settings = profile.settings_mut(id);
                    settings.width = next.x;
                    settings.height = next.y;
                    self.set_measured_size(id, next);
                }
            }
            return false;
        }

        if grip.drag_stopped
            && let Some(session) = self.resize.take().filter(|s| s.id == id)
        {
            if !session.size_changed {
                return false;
            }
            let settings = profile.settings_mut(id);
            settings.width = session.size.x;
            settings.height = session.size.y;
            settings.position =
                placement::normalized_position(viewport, rendered_size, margin, visible_top_left);
            self.set_measured_size(id, rendered_size);
            return true;
        }

        false
    }

    /// Shared post-paint for Warframe panels: resize first, else drag-move.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_warframe_panel(
        &mut self,
        id: WidgetId,
        viewport: Rect,
        margin: f32,
        profile: &mut WidgetProfile,
        rendered_size: Vec2,
        visible_top_left: Pos2,
        dragged: bool,
        drag_stopped: bool,
        resize: crate::widgets::chrome::ResizeGripOutcome,
    ) -> bool {
        let mut save = self.apply_resize_grip(
            id,
            viewport,
            margin,
            profile,
            rendered_size,
            visible_top_left,
            resize,
        );
        if !resize.dragging && !resize.drag_stopped {
            save |= self.finish_drag_only(
                id,
                viewport,
                margin,
                profile,
                rendered_size,
                visible_top_left,
                dragged,
                drag_stopped,
            );
        }
        save
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_drag_only(
        &mut self,
        id: WidgetId,
        viewport: Rect,
        margin: f32,
        profile: &mut WidgetProfile,
        size: Vec2,
        position: Pos2,
        dragged: bool,
        drag_stopped: bool,
    ) -> bool {
        // `size` must be the same value used for placement next frame (panel_size
        // from paint), not a fluctuating Area rect.
        self.set_measured_size(id, size);
        if dragged || drag_stopped {
            profile.settings_mut(id).position =
                placement::normalized_position(viewport, size, margin, position);
        }
        placement_save_requested(dragged, drag_stopped)
    }
}

pub fn placement_save_requested(dragged: bool, drag_stopped: bool) -> bool {
    !dragged && drag_stopped
}

const RESIZE_GRIP: f32 = 20.0;

fn pointer_near_resize_grip(ui: &egui::Ui, top_left: Pos2, panel_size: Vec2) -> bool {
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return false;
    };
    let panel = Rect::from_min_size(top_left, panel_size);
    Rect::from_min_size(
        panel.max - eframe::egui::vec2(RESIZE_GRIP, RESIZE_GRIP),
        eframe::egui::vec2(RESIZE_GRIP, RESIZE_GRIP),
    )
    .expand(6.0)
    .contains(pointer)
}

fn panel_min_width(id: WidgetId) -> f32 {
    match id {
        WidgetId::WarframeFissures => crate::widgets::chrome::FISSURE_PANEL_MIN_WIDTH,
        _ => WIDGET_PANEL_MIN,
    }
}

fn clamp_delta_at_limits(size: Vec2, mut delta: Vec2, min_width: f32) -> Vec2 {
    if size.x <= min_width + 0.5 && delta.x < 0.0 {
        delta.x = 0.0;
    }
    if size.x >= WIDGET_PANEL_MAX - 0.5 && delta.x > 0.0 {
        delta.x = 0.0;
    }
    if size.y <= WIDGET_PANEL_MIN_HEIGHT + 0.5 && delta.y < 0.0 {
        delta.y = 0.0;
    }
    if size.y >= WIDGET_PANEL_MAX - 0.5 && delta.y > 0.0 {
        delta.y = 0.0;
    }
    delta
}

fn size_meaningfully_changed(before: Vec2, after: Vec2) -> bool {
    (after.x - before.x).abs() > 0.5 || (after.y - before.y).abs() > 0.5
}

fn widget_index(id: WidgetId) -> usize {
    match id {
        WidgetId::Session => 0,
        WidgetId::Clock => 1,
        WidgetId::Performance => 2,
        WidgetId::ManualStopwatch => 3,
        WidgetId::Media => 4,
        WidgetId::Notes => 5,
        WidgetId::WarframeStatus => 6,
        WidgetId::WarframeFissures => 7,
        WidgetId::WarframeMarket => 8,
        WidgetId::WarframeSortie => 9,
        WidgetId::WarframeInvasions => 10,
    }
}
