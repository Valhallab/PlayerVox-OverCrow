use eframe::egui::{self, Color32, Layout, Sense, Stroke, Vec2, vec2};
use overcrow_config::WARFRAME_MARKET_QUERY_MAX_CHARS;
use overcrow_protocol::OverlayMode;

use super::chrome::{
    BODY_SIZE, META_SIZE, ResizeGripOutcome, accent_error, accent_ok, apply_scale, meta_text,
    panel_frame, resize_grip, title_text,
};
use crate::warframe::{
    MarketCommand, MarketOrder, MarketSnapshot, TradeSide, format_trade_line, format_whisper_line,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarketUiAction {
    Command(MarketCommand),
    CopyTrade { text: String, flash_id: String },
}

pub struct WarframeMarketResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub resize: ResizeGripOutcome,
    pub actions: Vec<MarketUiAction>,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_warframe_market(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    panel_size: Vec2,
    snapshot: &MarketSnapshot,
    draft_query: &mut String,
    copy_flash_id: Option<&str>,
    scale: f32,
    mode: OverlayMode,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> WarframeMarketResponse {
    let mut actions = Vec::new();
    let interactive = mode == OverlayMode::Interactive;
    let panel_size = super::chrome::clamp_panel_size(panel_size);
    let mut resize = ResizeGripOutcome::default();

    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("warframe-market-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(true)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            apply_scale(ui, scale);
            panel_frame(transparent_background).show(ui, |ui| {
                ui.set_min_size(panel_size);
                ui.set_max_size(panel_size);

                ui.horizontal(|ui| {
                    ui.label(title_text("MARKET"));
                    if snapshot.selected.is_some() && snapshot.selected_fetched_at_secs > 0 {
                        ui.label(meta_text(format!(
                            "auto refresh {}s",
                            crate::warframe::MARKET_ORDERS_REFRESH_SECS
                        )));
                    }
                });
                ui.add_space(4.0);

                if interactive {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(draft_query)
                                .char_limit(WARFRAME_MARKET_QUERY_MAX_CHARS)
                                .desired_width((panel_size.x - 168.0).max(100.0))
                                .hint_text("Item…"),
                        );
                        if ui.button("Search").clicked() {
                            actions.push(MarketUiAction::Command(MarketCommand::Search(
                                draft_query.clone(),
                            )));
                        }
                        if ui
                            .button("Clear")
                            .on_hover_text("Clear search and selection")
                            .clicked()
                        {
                            draft_query.clear();
                            actions.push(MarketUiAction::Command(MarketCommand::Clear));
                        }
                    });
                } else if snapshot.selected.is_none() {
                    ui.label(meta_text(
                        "Select an item in interactive mode to track prices.",
                    ));
                }

                if let Some(status) = &snapshot.status {
                    ui.label(meta_text(status.clone()));
                }
                if let Some(error) = &snapshot.error {
                    ui.colored_label(accent_error(), egui::RichText::new(error).size(BODY_SIZE));
                }

                let content_h = (panel_size.y - if interactive { 72.0 } else { 48.0 }).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt("market-scroll")
                    .max_height(content_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if interactive {
                            for item in &snapshot.results {
                                let selected = snapshot
                                    .selected
                                    .as_ref()
                                    .is_some_and(|selected| selected.slug == item.slug);
                                if ui
                                    .selectable_label(
                                        selected,
                                        egui::RichText::new(&item.name).size(BODY_SIZE),
                                    )
                                    .clicked()
                                {
                                    actions.push(MarketUiAction::Command(MarketCommand::Select(
                                        item.slug.clone(),
                                    )));
                                }
                            }
                        }

                        if let Some(selected) = &snapshot.selected {
                            if interactive {
                                ui.add_space(6.0);
                                ui.separator();
                            }
                            ui.label(
                                egui::RichText::new(&selected.name)
                                    .size(BODY_SIZE + 2.0)
                                    .strong()
                                    .color(Color32::from_gray(245)),
                            );

                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                price_stat(ui, "Min sell", selected.lowest_sell);
                                ui.add_space(16.0);
                                price_stat(ui, "Max buy", selected.highest_buy);
                                ui.add_space(16.0);
                                ui.label(
                                    egui::RichText::new(format!("{} orders", selected.order_count))
                                        .size(META_SIZE)
                                        .color(Color32::from_gray(170)),
                                );
                            });

                            ui.add_space(10.0);
                            // One grid for both sections so price/Whisper columns align globally.
                            paint_orders_combined(
                                ui,
                                &selected.top_sells,
                                &selected.top_buys,
                                &selected.name,
                                copy_flash_id,
                                interactive,
                                &mut actions,
                            );

                            if interactive {
                                ui.add_space(8.0);
                                ui.label(meta_text("Chat templates (no player)"));
                                ui.horizontal(|ui| {
                                    if let Some(price) = selected.lowest_sell {
                                        copy_or_check(
                                            ui,
                                            "tpl-wts",
                                            "Copy WTS (min)",
                                            copy_flash_id,
                                            format_trade_line(
                                                TradeSide::Sell,
                                                &selected.name,
                                                price,
                                            ),
                                            &mut actions,
                                        );
                                    }
                                    if let Some(price) = selected.highest_buy {
                                        copy_or_check(
                                            ui,
                                            "tpl-wtb",
                                            "Copy WTB (max)",
                                            copy_flash_id,
                                            format_trade_line(
                                                TradeSide::Buy,
                                                &selected.name,
                                                price,
                                            ),
                                            &mut actions,
                                        );
                                    }
                                });
                            }
                        }
                    });

                let panel_rect = ui.min_rect();
                resize = resize_grip(ui, panel_rect, mode == OverlayMode::Interactive);
            });
        });

    // Report the content panel size for placement (stable), not fluctuating Area rect.
    WarframeMarketResponse {
        size: panel_size,
        position: response.response.rect.min,
        dragged: response.response.dragged() && !resize.dragging,
        drag_stopped: response.response.drag_stopped() && !resize.dragging && !resize.drag_stopped,
        resize,
        actions,
    }
}

fn price_stat(ui: &mut egui::Ui, label: &str, price: Option<u32>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .size(META_SIZE)
                .color(Color32::from_gray(170)),
        );
        let value = price
            .map(|p| format!("{p}p"))
            .unwrap_or_else(|| "—".to_owned());
        ui.label(
            egui::RichText::new(value)
                .size(BODY_SIZE + 1.0)
                .strong()
                .color(Color32::from_gray(240)),
        );
    });
}

fn copy_or_check(
    ui: &mut egui::Ui,
    flash_id: &str,
    label: &str,
    active_flash: Option<&str>,
    text: String,
    actions: &mut Vec<MarketUiAction>,
) {
    if active_flash == Some(flash_id) {
        paint_check_mark(ui, accent_ok());
        return;
    }
    if ui.button(label).clicked() {
        actions.push(MarketUiAction::CopyTrade {
            text,
            flash_id: flash_id.to_owned(),
        });
    }
}

/// One grid for sellers + buyers so Plat/Whisper columns share widths globally.
fn paint_orders_combined(
    ui: &mut egui::Ui,
    sells: &[MarketOrder],
    buys: &[MarketOrder],
    item_name: &str,
    copy_flash_id: Option<&str>,
    interactive: bool,
    actions: &mut Vec<MarketUiAction>,
) {
    if sells.is_empty() && buys.is_empty() {
        ui.label(meta_text("No listed orders"));
        return;
    }

    let cols = if interactive { 3 } else { 2 };
    egui::Grid::new("market-orders-all")
        .num_columns(cols)
        .spacing([12.0, 4.0])
        .min_col_width(48.0)
        .show(ui, |ui| {
            ui.label(meta_text("Player"));
            ui.with_layout(Layout::top_down(egui::Align::Center), |ui| {
                ui.label(meta_text("Plat"));
            });
            if interactive {
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(meta_text("Action"));
                });
            }
            ui.end_row();

            paint_section_header_row(ui, "Sellers", interactive);
            if sells.is_empty() {
                paint_empty_row(ui, interactive);
            } else {
                for (index, order) in sells.iter().enumerate() {
                    paint_order_row(
                        ui,
                        format!("sell-{index}"),
                        order,
                        item_name,
                        TradeSide::Buy,
                        copy_flash_id,
                        interactive,
                        actions,
                    );
                }
            }

            paint_section_header_row(ui, "Buyers", interactive);
            if buys.is_empty() {
                paint_empty_row(ui, interactive);
            } else {
                for (index, order) in buys.iter().enumerate() {
                    paint_order_row(
                        ui,
                        format!("buy-{index}"),
                        order,
                        item_name,
                        TradeSide::Sell,
                        copy_flash_id,
                        interactive,
                        actions,
                    );
                }
            }
        });
}

fn paint_section_header_row(ui: &mut egui::Ui, title: &str, interactive: bool) {
    ui.label(
        egui::RichText::new(title)
            .size(META_SIZE)
            .strong()
            .color(Color32::from_gray(200)),
    );
    ui.label("");
    if interactive {
        ui.label("");
    }
    ui.end_row();
}

fn paint_empty_row(ui: &mut egui::Ui, interactive: bool) {
    ui.label(meta_text("—"));
    ui.label("");
    if interactive {
        ui.label("");
    }
    ui.end_row();
}

#[allow(clippy::too_many_arguments)]
fn paint_order_row(
    ui: &mut egui::Ui,
    flash_id: String,
    order: &MarketOrder,
    item_name: &str,
    your_intent: TradeSide,
    copy_flash_id: Option<&str>,
    interactive: bool,
    actions: &mut Vec<MarketUiAction>,
) {
    ui.horizontal(|ui| {
        paint_status_dot(ui, presence_color(order.presence.label()));
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(&order.trader)
                .size(BODY_SIZE)
                .strong()
                .color(Color32::from_gray(230)),
        );
    });

    ui.with_layout(Layout::top_down(egui::Align::Center), |ui| {
        ui.label(
            egui::RichText::new(format!("{}p", order.platinum))
                .monospace()
                .size(BODY_SIZE)
                .strong()
                .color(Color32::from_gray(235)),
        );
    });

    if interactive {
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if copy_flash_id == Some(flash_id.as_str()) {
                paint_check_mark(ui, accent_ok());
            } else if ui.small_button("Whisper").clicked() {
                actions.push(MarketUiAction::CopyTrade {
                    text: format_whisper_line(
                        your_intent,
                        &order.trader,
                        item_name,
                        order.platinum,
                    ),
                    flash_id,
                });
            }
        });
    }
    ui.end_row();
}

fn paint_status_dot(ui: &mut egui::Ui, color: Color32) {
    let size = vec2(9.0, 9.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), size.x * 0.42, color);
}

fn paint_check_mark(ui: &mut egui::Ui, color: Color32) {
    let size = vec2(16.0, 16.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let c = rect.center();
    let stroke = Stroke::new(2.0, color);
    ui.painter()
        .line_segment([c + vec2(-5.0, 0.0), c + vec2(-1.5, 4.0)], stroke);
    ui.painter()
        .line_segment([c + vec2(-1.5, 4.0), c + vec2(5.5, -4.0)], stroke);
}

fn presence_color(label: &str) -> Color32 {
    match label {
        "in game" => accent_ok(),
        "online" => Color32::from_rgb(180, 220, 140),
        "offline" => Color32::from_gray(120),
        _ => Color32::from_gray(150),
    }
}
