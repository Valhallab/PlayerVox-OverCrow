use eframe::egui::{self, Color32, FontId, Layout, Pos2, Sense, Stroke, Vec2, vec2};
use overcrow_config::WarframePrefs;
use overcrow_protocol::OverlayMode;

use super::chrome::{
    BODY_SIZE, META_SIZE, ResizeGripOutcome, accent_warn, apply_scale, meta_text, options_menu,
    panel_frame, panel_width_limits, report_content_panel_size, resize_grip, title_text,
};
use crate::warframe::{
    InvasionCompactLabel, InvasionMission, RewardLine, WarframeDerivedCache, format_node,
    invasion_done_key, invasion_on_watchlist,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvasionPrefsAction {
    ToggleHideCompleted,
    TogglePushDoneDown,
    ToggleCompact,
    ToggleWatchlist(String),
    ClearWatchlist,
    ToggleDone(String),
    ToggleResourceFilter(String),
    ClearResourceFilter,
}

pub struct WarframeInvasionsResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub resize: ResizeGripOutcome,
    pub actions: Vec<InvasionPrefsAction>,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_warframe_invasions(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    panel_size: Vec2,
    invasions: &[InvasionMission],
    invasion_indices: &[usize],
    reward_catalog: &[(String, String)],
    derived_cache: &mut WarframeDerivedCache,
    worldstate_revision: u64,
    prefs_revision: u64,
    prefs: &WarframePrefs,
    scale: f32,
    mode: OverlayMode,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> WarframeInvasionsResponse {
    let panel_size = super::chrome::clamp_panel_size(panel_size);
    let compact_labels = prefs.invasion_compact.then(|| {
        let effective_width = (panel_size.x - 32.0).max(80.0);
        derived_cache.compact_invasion_labels(
            worldstate_revision,
            prefs_revision,
            effective_width,
            |quantized_width| {
                invasion_indices
                    .iter()
                    .take(12)
                    .map(|index| {
                        compact_label(ui, &invasions[*index], prefs, quantized_width.max(80.0))
                    })
                    .collect()
            },
        )
    });
    let mut resize = ResizeGripOutcome::default();
    let mut actions = Vec::new();

    let viewport = ui.max_rect();
    let response = egui::Area::new(egui::Id::new("warframe-invasions-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(true)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            apply_scale(ui, scale);
            panel_frame(transparent_background).show(ui, |ui| {
                panel_width_limits(ui, panel_size.x);
                ui.set_max_height(panel_size.y);

                paint_header(
                    ui,
                    invasion_indices.len(),
                    mode,
                    prefs,
                    reward_catalog,
                    &mut actions,
                );
                ui.add_space(4.0);

                let header_h = 36.0 * scale;
                let body_max = (panel_size.y - header_h).max(64.0);

                if invasion_indices.is_empty() {
                    ui.label(meta_text("No active invasions"));
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("invasions-scroll")
                        .max_height(body_max)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.set_min_width(panel_size.x - 32.0);
                            for (row, index) in invasion_indices.iter().take(12).enumerate() {
                                let invasion = &invasions[*index];
                                ui.scope_builder(
                                    egui::UiBuilder::new().id(egui::Id::new((
                                        "warframe-invasion-row",
                                        &invasion.instance_id,
                                    ))),
                                    |ui| {
                                        paint_invasion_row(
                                            ui,
                                            invasion,
                                            compact_labels.as_ref().map(|labels| &labels[row]),
                                            prefs,
                                            mode,
                                            &mut actions,
                                        );
                                    },
                                );
                                ui.add_space(if prefs.invasion_compact { 3.0 } else { 8.0 });
                            }
                        });
                }

                let panel_rect = ui.min_rect();
                resize = resize_grip(ui, panel_rect, mode == OverlayMode::Interactive);
            });
        });

    let measured = response.response.rect.size().max(vec2(1.0, 1.0));
    WarframeInvasionsResponse {
        size: report_content_panel_size(panel_size, measured),
        position: response.response.rect.min,
        dragged: response.response.dragged() && !resize.dragging,
        drag_stopped: response.response.drag_stopped() && !resize.dragging && !resize.drag_stopped,
        resize,
        actions,
    }
}

fn paint_header(
    ui: &mut egui::Ui,
    count: usize,
    mode: OverlayMode,
    prefs: &WarframePrefs,
    reward_catalog: &[(String, String)],
    actions: &mut Vec<InvasionPrefsAction>,
) {
    ui.horizontal(|ui| {
        ui.label(title_text("INVASIONS"));
        ui.label(
            egui::RichText::new(format!("{count}"))
                .size(META_SIZE)
                .strong()
                .color(Color32::from_gray(180)),
        );
        if mode == OverlayMode::Interactive {
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                options_menu(ui, |ui| {
                    let mut hide = prefs.invasion_hide_completed;
                    if ui.checkbox(&mut hide, "Hide DE-completed").changed() {
                        actions.push(InvasionPrefsAction::ToggleHideCompleted);
                    }
                    let mut push_down = prefs.invasion_push_done_down;
                    if ui
                        .checkbox(&mut push_down, "Move finished to bottom")
                        .changed()
                    {
                        actions.push(InvasionPrefsAction::TogglePushDoneDown);
                    }
                    let mut compact = prefs.invasion_compact;
                    if ui.checkbox(&mut compact, "Compact mode").changed() {
                        actions.push(InvasionPrefsAction::ToggleCompact);
                    }
                    if !prefs.invasion_reward_watchlist.is_empty()
                        && ui.button("Clear watchlist").clicked()
                    {
                        actions.push(InvasionPrefsAction::ClearWatchlist);
                    }
                    if !reward_catalog.is_empty() {
                        ui.separator();
                        ui.label(
                            egui::RichText::new("Rewards")
                                .size(META_SIZE)
                                .strong()
                                .color(Color32::from_gray(180)),
                        );
                        for (key, label) in reward_catalog {
                            let mut checked = prefs.invasion_resource_checked(key);
                            if ui
                                .checkbox(&mut checked, egui::RichText::new(label).size(META_SIZE))
                                .changed()
                            {
                                actions
                                    .push(InvasionPrefsAction::ToggleResourceFilter(key.clone()));
                            }
                        }
                        if !prefs.invasion_resource_filter.is_empty()
                            && ui.small_button("Show all rewards").clicked()
                        {
                            actions.push(InvasionPrefsAction::ClearResourceFilter);
                        }
                    }
                });
            });
        }
    });
}

fn paint_invasion_row(
    ui: &mut egui::Ui,
    invasion: &InvasionMission,
    compact_label: Option<&InvasionCompactLabel>,
    prefs: &WarframePrefs,
    mode: OverlayMode,
    actions: &mut Vec<InvasionPrefsAction>,
) {
    if prefs.invasion_compact {
        paint_invasion_row_compact(
            ui,
            invasion,
            compact_label.expect("compact rows have revision-keyed labels"),
            prefs,
            mode,
            actions,
        );
        return;
    }

    let done_key = invasion_done_key(&invasion.instance_id);
    let done = prefs.activity_is_done(&done_key);
    let watched = invasion_on_watchlist(invasion, prefs);
    let node_color = if watched {
        accent_warn()
    } else if done {
        Color32::from_gray(140)
    } else {
        Color32::from_gray(220)
    };

    ui.horizontal(|ui| {
        let mut checked = done;
        if ui
            .add_enabled(
                mode == OverlayMode::Interactive,
                egui::Checkbox::new(&mut checked, ""),
            )
            .changed()
        {
            actions.push(InvasionPrefsAction::ToggleDone(done_key));
        }
        let mut node = egui::RichText::new(format_node(&invasion.node))
            .size(BODY_SIZE)
            .color(node_color);
        if done {
            node = node.strikethrough();
        } else {
            node = node.strong();
        }
        ui.label(node);
        if done {
            // Compact one-line summary when finished.
            let rewards = [
                invasion.attacker_reward.as_ref().map(|r| r.label.as_str()),
                invasion.defender_reward.as_ref().map(|r| r.label.as_str()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            if !rewards.is_empty() {
                ui.label(
                    egui::RichText::new(rewards)
                        .size(META_SIZE)
                        .color(Color32::from_gray(130)),
                );
            }
        }
    });

    if done {
        return;
    }

    let ratio = invasion.attacker_bar_ratio().unwrap_or(0.5);
    let percent = invasion.progress_percent().unwrap_or(0);
    let attacker_color = faction_color(&invasion.attacker_faction);
    let defender_color = faction_color(&invasion.defender_faction);
    paint_dual_progress(
        ui,
        ratio,
        percent,
        &invasion.attacker_faction,
        &invasion.defender_faction,
        attacker_color,
        defender_color,
    );
    if invasion.attacker_reward.is_some() || invasion.defender_reward.is_some() {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            paint_reward_compact(
                ui,
                invasion.attacker_reward.as_ref(),
                attacker_color,
                prefs,
                mode,
                actions,
            );
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                paint_reward_compact(
                    ui,
                    invasion.defender_reward.as_ref(),
                    defender_color,
                    prefs,
                    mode,
                    actions,
                );
            });
        });
    }
}

/// Checkbox outside; dual progress bar holds left reward, node (center), right reward.
fn paint_invasion_row_compact(
    ui: &mut egui::Ui,
    invasion: &InvasionMission,
    label: &InvasionCompactLabel,
    prefs: &WarframePrefs,
    mode: OverlayMode,
    actions: &mut Vec<InvasionPrefsAction>,
) {
    let done_key = invasion_done_key(&invasion.instance_id);
    let done = prefs.activity_is_done(&done_key);
    let watched = invasion_on_watchlist(invasion, prefs);
    let ratio = invasion.attacker_bar_ratio().unwrap_or(0.5);
    let attacker_color = faction_color(&invasion.attacker_faction);
    let defender_color = faction_color(&invasion.defender_faction);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let mut checked = done;
        if ui
            .add_enabled(
                mode == OverlayMode::Interactive,
                egui::Checkbox::new(&mut checked, ""),
            )
            .changed()
        {
            actions.push(InvasionPrefsAction::ToggleDone(done_key));
        }

        let height = 24.0;
        let width = ui.available_width().max(80.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        paint_compact_progress_bar(
            ui,
            rect,
            ratio,
            done,
            attacker_color,
            defender_color,
            invasion.attacker_reward.as_ref(),
            invasion.defender_reward.as_ref(),
            label,
            watched,
            prefs,
            mode,
            actions,
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn paint_compact_progress_bar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    attacker_ratio: f32,
    done: bool,
    attacker_color: Color32,
    defender_color: Color32,
    attacker_reward: Option<&RewardLine>,
    defender_reward: Option<&RewardLine>,
    label: &InvasionCompactLabel,
    watched: bool,
    prefs: &WarframePrefs,
    mode: OverlayMode,
    actions: &mut Vec<InvasionPrefsAction>,
) {
    let painter = ui.painter();
    let rounding = 5.0;

    if done {
        painter.rect_filled(rect, rounding, Color32::from_black_alpha(90));
    } else {
        paint_dual_progress_fill(
            painter,
            rect,
            rounding,
            attacker_ratio,
            attacker_color,
            defender_color,
        );
    }
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0, Color32::from_white_alpha(40)),
        egui::StrokeKind::Inside,
    );

    let pad = 8.0;
    let left_watched = attacker_reward.is_some_and(|r| prefs.invasion_watchlisted(&r.item_key));
    let right_watched = defender_reward.is_some_and(|r| prefs.invasion_watchlisted(&r.item_key));
    let font = FontId::proportional(META_SIZE);
    let node_font = FontId::proportional(META_SIZE + if done { 0.0 } else { 1.0 });

    if let Some(label) = &label.attacker {
        painter.text(
            Pos2::new(rect.left() + pad, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            compact_label_color(done, left_watched, 245),
        );
    }
    if let Some(label) = &label.defender {
        painter.text(
            Pos2::new(rect.right() - pad, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            label,
            font,
            compact_label_color(done, right_watched, 245),
        );
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        &label.node,
        node_font,
        compact_label_color(done, watched, 250),
    );

    if mode == OverlayMode::Interactive && !done {
        let third = rect.width() / 3.0;
        if let Some(reward) = attacker_reward {
            let zone =
                egui::Rect::from_min_max(rect.min, Pos2::new(rect.left() + third, rect.bottom()));
            watchlist_click_zone(ui, zone, "inv-compact-left", reward, left_watched, actions);
        }
        if let Some(reward) = defender_reward {
            let zone =
                egui::Rect::from_min_max(Pos2::new(rect.right() - third, rect.top()), rect.max);
            watchlist_click_zone(
                ui,
                zone,
                "inv-compact-right",
                reward,
                right_watched,
                actions,
            );
        }
    }
}

fn compact_label(
    ui: &egui::Ui,
    invasion: &InvasionMission,
    prefs: &WarframePrefs,
    effective_width: f32,
) -> InvasionCompactLabel {
    let pad = 8.0;
    let side_max = ((effective_width - 2.0 * pad) * 0.30).max(40.0);
    let font = FontId::proportional(META_SIZE);
    let done = prefs.activity_is_done(&invasion_done_key(&invasion.instance_id));
    let node_font = FontId::proportional(META_SIZE + if done { 0.0 } else { 1.0 });
    InvasionCompactLabel {
        attacker: reward_label(invasion.attacker_reward.as_ref())
            .map(|label| clip_label(ui, &label, &font, side_max)),
        defender: reward_label(invasion.defender_reward.as_ref())
            .map(|label| clip_label(ui, &label, &font, side_max)),
        node: clip_label(
            ui,
            &format_node(&invasion.node),
            &node_font,
            (effective_width * 0.40).max(48.0),
        ),
    }
}

fn paint_dual_progress_fill(
    painter: &egui::Painter,
    rect: egui::Rect,
    rounding: f32,
    attacker_ratio: f32,
    attacker_color: Color32,
    defender_color: Color32,
) {
    painter.rect_filled(rect, rounding, dim_color(defender_color, 0.40));
    let fill_w = (rect.width() * attacker_ratio.clamp(0.0, 1.0)).max(if attacker_ratio > 0.0 {
        2.0
    } else {
        0.0
    });
    if fill_w > 0.0 {
        let fill = egui::Rect::from_min_size(rect.min, vec2(fill_w, rect.height()));
        painter.rect_filled(fill, rounding, dim_color(attacker_color, 0.90));
    }
}

fn compact_label_color(done: bool, highlight: bool, normal_gray: u8) -> Color32 {
    if done {
        Color32::from_gray(130)
    } else if highlight {
        accent_warn()
    } else {
        Color32::from_gray(normal_gray)
    }
}

fn watchlist_click_zone(
    ui: &mut egui::Ui,
    zone: egui::Rect,
    salt: &str,
    reward: &RewardLine,
    watched: bool,
    actions: &mut Vec<InvasionPrefsAction>,
) {
    let response = ui.interact(
        zone,
        ui.id().with((salt, reward.item_key.as_str())),
        Sense::click(),
    );
    if response.clicked() {
        actions.push(InvasionPrefsAction::ToggleWatchlist(
            reward.item_key.clone(),
        ));
    }
    response.on_hover_text(if watched {
        "Remove from watchlist"
    } else {
        "Add to watchlist"
    });
}

fn reward_label(reward: Option<&RewardLine>) -> Option<String> {
    reward.map(|reward| {
        if reward.count > 1 {
            format!("{} ×{}", reward.label, reward.count)
        } else {
            reward.label.clone()
        }
    })
}

fn clip_label(ui: &egui::Ui, label: &str, font: &FontId, max_width: f32) -> String {
    let painter = ui.painter();
    let text_width = |text: &str| {
        painter
            .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if text_width(label) <= max_width {
        return label.to_owned();
    }

    let mut low = 0;
    let mut high = label.chars().count();
    let mut best = "…".to_owned();
    while low <= high {
        let mid = (low + high) / 2;
        let candidate: String = label.chars().take(mid).collect::<String>() + "…";
        if text_width(&candidate) <= max_width {
            best = candidate;
            low = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    best
}

fn paint_dual_progress(
    ui: &mut egui::Ui,
    attacker_ratio: f32,
    percent: u8,
    attacker: &str,
    defender: &str,
    attacker_color: Color32,
    defender_color: Color32,
) {
    let height = 22.0;
    let width = ui.available_width().max(80.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter();
    let rounding = 5.0;

    paint_dual_progress_fill(
        painter,
        rect,
        rounding,
        attacker_ratio,
        attacker_color,
        defender_color,
    );
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0, Color32::from_white_alpha(40)),
        egui::StrokeKind::Inside,
    );

    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{percent}%"),
        FontId::proportional(META_SIZE + 1.0),
        Color32::from_gray(245),
    );
    painter.text(
        Pos2::new(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        attacker,
        FontId::proportional(META_SIZE - 1.0),
        Color32::from_gray(240),
    );
    painter.text(
        Pos2::new(rect.right() - 8.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        defender,
        FontId::proportional(META_SIZE - 1.0),
        Color32::from_gray(240),
    );
}

fn paint_reward_compact(
    ui: &mut egui::Ui,
    reward: Option<&RewardLine>,
    faction_color: Color32,
    prefs: &WarframePrefs,
    mode: OverlayMode,
    actions: &mut Vec<InvasionPrefsAction>,
) {
    let Some(reward) = reward else {
        return;
    };
    let watched = prefs.invasion_watchlisted(&reward.item_key);
    let label = reward_label(Some(reward)).unwrap_or_default();
    let color = if watched {
        accent_warn()
    } else {
        faction_color
    };
    let text = egui::RichText::new(label)
        .size(META_SIZE)
        .strong()
        .color(color);
    if mode == OverlayMode::Interactive {
        if ui
            .add(egui::Label::new(text).sense(Sense::click()))
            .on_hover_text(if watched {
                "Remove from watchlist"
            } else {
                "Add to watchlist"
            })
            .clicked()
        {
            actions.push(InvasionPrefsAction::ToggleWatchlist(
                reward.item_key.clone(),
            ));
        }
    } else {
        ui.label(text);
    }
}

fn faction_color(name: &str) -> Color32 {
    match name {
        "Grineer" => Color32::from_rgb(120, 190, 110),
        "Corpus" => Color32::from_rgb(90, 180, 240),
        "Infested" => Color32::from_rgb(170, 120, 220),
        "Orokin" => Color32::from_rgb(220, 200, 120),
        "Narmer" => Color32::from_rgb(220, 160, 70),
        "Sentient" => Color32::from_rgb(120, 210, 200),
        _ => Color32::from_gray(160),
    }
}

fn dim_color(color: Color32, factor: f32) -> Color32 {
    let factor = factor.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (f32::from(color.r()) * factor) as u8,
        (f32::from(color.g()) * factor) as u8,
        (f32::from(color.b()) * factor) as u8,
        255,
    )
}

pub fn apply_invasion_prefs_action(
    prefs: &mut WarframePrefs,
    action: InvasionPrefsAction,
    available_resource_keys: &[String],
) {
    match action {
        InvasionPrefsAction::ToggleHideCompleted => {
            prefs.invasion_hide_completed = !prefs.invasion_hide_completed;
        }
        InvasionPrefsAction::TogglePushDoneDown => {
            prefs.invasion_push_done_down = !prefs.invasion_push_done_down;
        }
        InvasionPrefsAction::ToggleCompact => {
            prefs.invasion_compact = !prefs.invasion_compact;
        }
        InvasionPrefsAction::ToggleWatchlist(key) => {
            prefs.toggle_invasion_watchlist(&key);
        }
        InvasionPrefsAction::ClearWatchlist => {
            prefs.invasion_reward_watchlist.clear();
        }
        InvasionPrefsAction::ToggleDone(key) => {
            prefs.toggle_activity_done(&key);
        }
        InvasionPrefsAction::ToggleResourceFilter(key) => {
            prefs.toggle_invasion_resource_filter(&key, available_resource_keys);
        }
        InvasionPrefsAction::ClearResourceFilter => {
            prefs.invasion_resource_filter.clear();
        }
    }
}
