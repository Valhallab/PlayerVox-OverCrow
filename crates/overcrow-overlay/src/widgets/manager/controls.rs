use eframe::egui::{self, Popup};
use overcrow_config::{WidgetId, WidgetProfile};

use super::{WidgetManager, layout::widget_index};
use crate::widgets::{
    CatalogAction,
    chrome::{
        ToolbarIcon, eyebrow_text, toolbar_icon_button, widget_toolbar_hover_rect,
        widget_toolbar_rect,
    },
};

impl WidgetManager {
    pub fn begin_widget_frame(&mut self) {
        self.visible_rects.fill(None);
        self.visible_order.clear();
    }

    pub fn clear_runtime_position(&mut self, id: WidgetId) {
        self.runtime_anchors[widget_index(id)] = None;
    }

    pub fn clear_runtime_size(&mut self, id: WidgetId) {
        let index = widget_index(id);
        self.measured_sizes[0][index] = egui::Vec2::ZERO;
        self.measured_sizes[1][index] = egui::Vec2::ZERO;
    }

    pub fn clear_runtime_geometry(&mut self) {
        self.measured_sizes = [[egui::Vec2::ZERO; WidgetId::COUNT]; 2];
        self.runtime_anchors.fill(None);
        self.resize = None;
    }

    pub fn paint_widget_controls(
        &mut self,
        context: &egui::Context,
        viewport: egui::Rect,
        profile: &WidgetProfile,
        mut paint_widget_options: impl FnMut(WidgetId, &mut egui::Ui),
    ) -> Vec<CatalogAction> {
        let popup_open = self
            .toolbar_popup_id
            .is_some_and(|popup_id| Popup::is_id_open(context, popup_id));
        if !popup_open {
            self.toolbar_open = None;
            self.toolbar_popup_id = None;
            self.pending_scales.fill(None);
        }

        let pointer = context.pointer_hover_pos();
        let hovered = self.visible_order.iter().rev().copied().find(|id| {
            self.visible_rects[widget_index(*id)].is_some_and(|widget| {
                pointer.is_some_and(|pointer| {
                    widget_toolbar_hover_rect(widget, viewport).contains(pointer)
                })
            })
        });
        let active = self.toolbar_open.filter(|_| popup_open).or(hovered);
        let Some(id) = active else {
            return Vec::new();
        };
        let Some(widget_rect) = self.visible_rects[widget_index(id)] else {
            return Vec::new();
        };

        let settings = profile.settings(id);
        let toolbar_rect = widget_toolbar_rect(widget_rect, viewport);
        let mut actions = Vec::new();
        egui::Area::new(egui::Id::new(("widget-toolbar", widget_index(id))))
            .order(egui::Order::Foreground)
            .fixed_pos(toolbar_rect.min)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let passive_icon = if settings.show_in_passive {
                        ToolbarIcon::PassiveVisible
                    } else {
                        ToolbarIcon::PassiveHidden
                    };
                    let passive_label = if settings.show_in_passive {
                        "Hide in passive mode"
                    } else {
                        "Show in passive mode"
                    };
                    if toolbar_icon_button(ui, passive_icon, passive_label).clicked() {
                        actions.push(CatalogAction::SetPassive(id, !settings.show_in_passive));
                    }

                    let options = toolbar_icon_button(ui, ToolbarIcon::Options, "Widget options");
                    let popup_id = Popup::default_response_id(&options);
                    let menu = Popup::menu(&options)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .width(240.0)
                        .show(|ui| {
                            ui.set_min_width(220.0);
                            ui.spacing_mut().item_spacing.y = 6.0;
                            ui.label(eyebrow_text("OPTIONS"));
                            ui.menu_button("Widget options", |ui| {
                                ui.set_min_width(220.0);
                                let mut transparent = settings.transparent_background;
                                if ui
                                    .checkbox(&mut transparent, "Transparent background")
                                    .changed()
                                {
                                    actions.push(CatalogAction::SetTransparentBackground(
                                        id,
                                        transparent,
                                    ));
                                }
                                if ui.button("Reset content size").clicked() {
                                    actions.push(CatalogAction::ResetSize(id));
                                    ui.close();
                                }
                                if ui.button("Reset position").clicked() {
                                    actions.push(CatalogAction::ResetPosition(id));
                                    ui.close();
                                }

                                ui.separator();
                                let index = widget_index(id);
                                let mut scale =
                                    self.pending_scales[index].unwrap_or(settings.scale);
                                let slider = ui.add(
                                    egui::Slider::new(&mut scale, 0.75..=1.75)
                                        .text("Content scale")
                                        .custom_formatter(|value, _| {
                                            format!("{:.0}%", value * 100.0)
                                        }),
                                );
                                if slider.changed() {
                                    self.pending_scales[index] = Some(scale);
                                }
                                if slider.drag_stopped() || (slider.changed() && !slider.dragged())
                                {
                                    self.pending_scales[index] = None;
                                    actions.push(CatalogAction::SetScale(id, scale));
                                }
                            });
                            paint_widget_options(id, ui);
                        });
                    if menu.is_some() {
                        self.toolbar_open = Some(id);
                        self.toolbar_popup_id = Some(popup_id);
                    }

                    let disable = toolbar_icon_button(ui, ToolbarIcon::Disable, "Disable widget");
                    if disable.clicked() {
                        actions.push(CatalogAction::SetEnabled(id, false));
                    }
                });
            });

        actions
    }
}
