use eframe::egui::{self, Pos2, Rect, Vec2};
use overcrow_config::{
    WIDGET_PANEL_MAX, WIDGET_PANEL_MIN, WIDGET_PANEL_MIN_HEIGHT, WidgetId, WidgetProfile,
};
use overcrow_protocol::OverlayMode;

use crate::placement;

use super::{ResizeSession, WidgetManager, widget_draggable};

#[derive(Clone, Copy)]
enum ResizeAxis {
    Both,
    Horizontal,
}

impl WidgetManager {
    pub(super) fn measured_size(&self, id: WidgetId, mode: OverlayMode) -> Vec2 {
        self.measured_sizes[mode_index(mode)][widget_index(id)]
    }

    pub(super) fn set_measured_size(&mut self, id: WidgetId, mode: OverlayMode, size: Vec2) {
        self.measured_sizes[mode_index(mode)][widget_index(id)] = size;
    }

    pub(super) fn request_repaint_if_size_changed(
        &self,
        ui: &egui::Ui,
        id: WidgetId,
        mode: OverlayMode,
        size: Vec2,
    ) {
        if size_meaningfully_changed(self.measured_size(id, mode), size) {
            ui.ctx().request_repaint();
        }
    }

    pub(crate) fn sync_interaction_state(
        &mut self,
        mode: OverlayMode,
        active_game: bool,
        pointer_down: bool,
    ) {
        let resize_owner_visible = self
            .resize
            .is_none_or(|session| self.visible_rects[widget_index(session.id)].is_some());
        if mode != OverlayMode::Interactive
            || !active_game
            || !self.interaction_enabled
            || !pointer_down
            || !resize_owner_visible
        {
            self.resize = None;
        }
        if !active_game {
            self.runtime_anchors.fill(None);
        }
    }

    pub(super) fn screen_position(
        &self,
        id: WidgetId,
        mode: OverlayMode,
        viewport: Rect,
        margin: f32,
        profile: &WidgetProfile,
    ) -> Pos2 {
        // During resize, freeze the absolute top-left.
        if mode == OverlayMode::Interactive
            && let Some(session) = self.resize
            && session.id == id
        {
            return session.anchor;
        }
        // Prefer last measured size when available (tests + post-drag); otherwise
        // fall back to the configured size for resizable widgets.
        let measured = self.measured_size(id, mode);
        if let Some(anchor) = self.runtime_anchors[widget_index(id)] {
            // A newly entered mode has no measurement yet. Preserve the exact
            // visible anchor until its real content size is known; using the
            // configured fallback here causes content-driven widgets to jump.
            let clamp_size = if measured.x > 1.0 && measured.y > 1.0 {
                measured
            } else {
                Vec2::splat(1.0)
            };
            return clamp_top_left(viewport, clamp_size, margin, anchor);
        }
        let size = if measured.x > 1.0 && measured.y > 1.0 {
            measured
        } else {
            let (w, h) = profile.settings(id).effective_panel_size(id);
            eframe::egui::vec2(w.max(1.0), h.max(1.0))
        };
        placement::screen_position(viewport, size, margin, profile.settings(id).position)
    }

    pub(super) fn panel_size(
        &self,
        id: WidgetId,
        mode: OverlayMode,
        profile: &WidgetProfile,
    ) -> Vec2 {
        if mode == OverlayMode::Interactive
            && let Some(session) = self.resize
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
        if !self.interaction_enabled {
            return false;
        }
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
        mode: OverlayMode,
        rendered_size: Vec2,
        visible_top_left: Pos2,
        grip: crate::widgets::chrome::ResizeGripOutcome,
        axis: ResizeAxis,
    ) -> bool {
        if grip.drag_cancelled {
            if self.resize.is_some_and(|session| session.id == id) {
                self.resize = None;
            }
            return false;
        }
        if !grip.dragging && !grip.drag_stopped {
            return false;
        }

        if self.resize.is_none() {
            let configured = self.panel_size(id, mode, profile);
            let size = eframe::egui::vec2(
                if configured.x > 1.0 {
                    configured.x
                } else {
                    rendered_size.x
                },
                if configured.y > 1.0 {
                    configured.y
                } else {
                    rendered_size.y
                },
            );
            self.resize = Some(ResizeSession {
                id,
                anchor: visible_top_left,
                size,
                size_changed: false,
            });
        }
        if let Some(session) = self.resize.as_mut().filter(|session| session.id == id) {
            let min_w = panel_min_width(id);
            let mut delta = clamp_delta_at_limits(session.size, grip.drag_delta, min_w);
            if matches!(axis, ResizeAxis::Horizontal) {
                delta.y = 0.0;
            }
            let next = crate::widgets::chrome::clamp_panel_size_min(session.size + delta, min_w);
            if size_meaningfully_changed(session.size, next) {
                session.size = next;
                session.size_changed = true;
            }
        }

        if !grip.drag_stopped {
            return false;
        }
        let session = match self.resize {
            Some(session) if session.id == id => {
                self.resize = None;
                session
            }
            _ => return false,
        };
        if !session.size_changed {
            return false;
        }

        let placement_size = {
            let settings = profile.settings_mut(id);
            settings.width = session.size.x;
            if matches!(axis, ResizeAxis::Both) {
                settings.height = session.size.y;
            }
            let (width, height) = settings.effective_panel_size(id);
            eframe::egui::vec2(width.max(1.0), height.max(1.0))
        };
        profile.settings_mut(id).position =
            placement::normalized_position(viewport, placement_size, margin, session.anchor);
        self.runtime_anchors[widget_index(id)] = Some(session.anchor);
        true
    }

    /// Shared post-paint for resizable panels: resize first, else drag-move.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_resizable_panel(
        &mut self,
        id: WidgetId,
        mode: OverlayMode,
        viewport: Rect,
        margin: f32,
        profile: &mut WidgetProfile,
        rendered_size: Vec2,
        visible_top_left: Pos2,
        dragged: bool,
        drag_stopped: bool,
        resize: crate::widgets::chrome::ResizeGripOutcome,
    ) -> bool {
        self.record_visible_rect(id, visible_top_left, rendered_size);
        let mut save = self.apply_resize_grip(
            id,
            viewport,
            margin,
            profile,
            mode,
            rendered_size,
            visible_top_left,
            resize,
            ResizeAxis::Both,
        );
        if !resize.dragging && !resize.drag_stopped && !resize.drag_cancelled {
            save |= self.finish_drag_only(
                id,
                mode,
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
    pub(super) fn finish_width_resizable_panel(
        &mut self,
        id: WidgetId,
        mode: OverlayMode,
        viewport: Rect,
        margin: f32,
        profile: &mut WidgetProfile,
        rendered_size: Vec2,
        visible_top_left: Pos2,
        dragged: bool,
        drag_stopped: bool,
        resize: crate::widgets::chrome::ResizeGripOutcome,
    ) -> bool {
        self.record_visible_rect(id, visible_top_left, rendered_size);
        let mut save = self.apply_resize_grip(
            id,
            viewport,
            margin,
            profile,
            mode,
            rendered_size,
            visible_top_left,
            resize,
            ResizeAxis::Horizontal,
        );
        if !resize.dragging && !resize.drag_stopped && !resize.drag_cancelled {
            save |= self.finish_drag_only(
                id,
                mode,
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
        mode: OverlayMode,
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
        self.set_measured_size(id, mode, size);
        self.record_visible_rect(id, position, size);
        self.runtime_anchors[widget_index(id)] = Some(position);
        if placement_save_requested(dragged, drag_stopped) {
            profile.settings_mut(id).position =
                placement::normalized_position(viewport, size, margin, position);
        }
        placement_save_requested(dragged, drag_stopped)
    }

    pub(super) fn record_visible_rect(&mut self, id: WidgetId, position: Pos2, size: Vec2) {
        self.visible_rects[widget_index(id)] = Some(Rect::from_min_size(position, size));
        if self.visible_order.len() < WidgetId::ALL.len() && !self.visible_order.contains(&id) {
            self.visible_order.push(id);
        }
    }
}

fn mode_index(mode: OverlayMode) -> usize {
    match mode {
        OverlayMode::Passive => 0,
        OverlayMode::Interactive => 1,
    }
}

pub fn placement_save_requested(dragged: bool, drag_stopped: bool) -> bool {
    !dragged && drag_stopped
}

fn pointer_near_resize_grip(ui: &egui::Ui, top_left: Pos2, panel_size: Vec2) -> bool {
    // A compositor may batch the press and the first motion into one egui
    // frame. Use the press origin while the button is held: the current
    // pointer can already be outside the small grip, which would otherwise
    // re-enable the movable parent Area and let it steal the resize gesture.
    let pointer = ui.input(|input| {
        input
            .pointer
            .primary_down()
            .then(|| input.pointer.press_origin())
            .flatten()
            .or_else(|| input.pointer.interact_pos())
    });
    let Some(pointer) = pointer else {
        return false;
    };
    let panel = Rect::from_min_size(top_left, panel_size);
    let grip = crate::widgets::chrome::resize_grip_rect(panel);
    // Keep the original 26 px inner and 6 px outer move guard around the
    // visible 18 px grip.
    Rect::from_min_max(grip.min - Vec2::splat(8.0), grip.max + Vec2::splat(6.0)).contains(pointer)
}

fn panel_min_width(id: WidgetId) -> f32 {
    match id {
        WidgetId::Performance => 300.0,
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

fn clamp_top_left(viewport: Rect, size: Vec2, margin: f32, position: Pos2) -> Pos2 {
    let min = viewport.min + Vec2::splat(margin);
    let max = viewport.max - Vec2::splat(margin) - size;
    Pos2::new(
        position.x.clamp(min.x, max.x.max(min.x)),
        position.y.clamp(min.y, max.y.max(min.y)),
    )
}

pub(super) fn widget_index(id: WidgetId) -> usize {
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
        WidgetId::TwitchChat => 11,
    }
}
