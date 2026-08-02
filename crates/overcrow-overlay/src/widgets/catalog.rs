use std::{fmt, io};

use eframe::egui::{self, Color32, Stroke, Vec2};
use overcrow_config::{
    DISCORD_PARTICIPANT_LIMIT_MAX, DISCORD_PARTICIPANT_LIMIT_MIN, DiscordVoiceAlignment,
    PerformanceLayout, WidgetId, WidgetProfile, settings_save_was_committed,
};
use overcrow_protocol::OverlayMode;

use super::{
    BUILTIN_WIDGETS, WidgetCategory, WidgetDescriptor,
    chrome::{
        ACCENT, PANEL_STROKE, SURFACE_RAISED, TEXT_MUTED, TEXT_PRIMARY, eyebrow_text, meta_text,
        paint_widget_glyph, title_text,
    },
};

pub const CATALOG_ERROR_MAX_CHARS: usize = 180;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CatalogLayout {
    pub width: f32,
    pub max_height: f32,
    pub columns: usize,
}

impl CatalogLayout {
    pub fn for_viewport(viewport: Vec2) -> Self {
        let width = (viewport.x - 72.0).clamp(200.0, 840.0).min(viewport.x);
        let max_height = (viewport.y - 170.0).clamp(120.0, 640.0).min(viewport.y);
        let columns = usize::from(width >= 680.0) + 1;
        Self {
            width,
            max_height,
            columns,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CatalogAction {
    SetEnabled(WidgetId, bool),
    SetPassive(WidgetId, bool),
    SetTransparentBackground(WidgetId, bool),
    SetScale(WidgetId, f32),
    SetClockDateVisible(bool),
    SetPerformanceLayout(PerformanceLayout),
    SetNotesNoteVisible(bool),
    SetNotesChecklistVisible(bool),
    SetDiscordParticipantLimit(u8),
    SetDiscordAlignment(DiscordVoiceAlignment),
    ResetSize(WidgetId),
    ResetPosition(WidgetId),
}

impl CatalogAction {
    pub const fn widget_id(self) -> WidgetId {
        match self {
            Self::SetEnabled(id, _)
            | Self::SetPassive(id, _)
            | Self::SetTransparentBackground(id, _)
            | Self::SetScale(id, _)
            | Self::ResetSize(id)
            | Self::ResetPosition(id) => id,
            Self::SetClockDateVisible(_) => WidgetId::Clock,
            Self::SetPerformanceLayout(_) => WidgetId::Performance,
            Self::SetNotesNoteVisible(_) | Self::SetNotesChecklistVisible(_) => WidgetId::Notes,
            Self::SetDiscordParticipantLimit(_) | Self::SetDiscordAlignment(_) => {
                WidgetId::DiscordVoice
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogCommit {
    pub reload_widget_settings: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogActionOutcome {
    Durable(CatalogCommit),
    CommittedWithWarning {
        commit: CatalogCommit,
        message: String,
    },
    RolledBack {
        message: String,
        category: CatalogFailureCategory,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogFailureCategory {
    Validation,
    Filesystem,
}

impl CatalogFailureCategory {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Filesystem => "filesystem",
        }
    }
}

pub fn catalog_visible(mode: OverlayMode, active_game: bool, open: bool) -> bool {
    active_game && mode == OverlayMode::Interactive && open
}

pub fn apply_catalog_action(
    profile: &mut WidgetProfile,
    action: CatalogAction,
    save: impl FnOnce(&WidgetProfile) -> io::Result<()>,
) -> CatalogActionOutcome {
    let previous = profile.clone();
    let mut candidate = previous.clone();
    let manual_enabled_before = candidate.manual_stopwatch.enabled;

    match action {
        CatalogAction::SetEnabled(id, enabled) => candidate.settings_mut(id).enabled = enabled,
        CatalogAction::SetPassive(id, visible) => {
            candidate.settings_mut(id).show_in_passive = visible;
        }
        CatalogAction::SetTransparentBackground(id, transparent) => {
            candidate.settings_mut(id).transparent_background = transparent;
        }
        CatalogAction::SetScale(id, scale) => {
            candidate.settings_mut(id).scale = scale;
        }
        CatalogAction::SetClockDateVisible(visible) => {
            candidate.clock_display.show_date = visible;
        }
        CatalogAction::SetPerformanceLayout(layout) => {
            candidate.performance_display.layout = layout;
        }
        CatalogAction::SetNotesNoteVisible(visible) => {
            candidate.notes_display.show_note = visible;
        }
        CatalogAction::SetNotesChecklistVisible(visible) => {
            candidate.notes_display.show_checklist = visible;
        }
        CatalogAction::SetDiscordParticipantLimit(limit) => {
            candidate.discord_voice_display.participant_limit = limit;
        }
        CatalogAction::SetDiscordAlignment(alignment) => {
            candidate.discord_voice_display.alignment = alignment;
        }
        CatalogAction::ResetSize(id) => {
            let (width, height) = id.default_panel_size();
            let settings = candidate.settings_mut(id);
            settings.width = width;
            settings.height = height;
        }
        CatalogAction::ResetPosition(id) => {
            candidate.settings_mut(id).position = id.default_position();
        }
    }

    let reload_widget_settings = matches!(
        action,
        CatalogAction::SetEnabled(WidgetId::ManualStopwatch, _)
    ) && candidate.manual_stopwatch.enabled != manual_enabled_before;
    *profile = candidate;
    persist_profile_change_with_commit(
        profile,
        previous,
        CatalogCommit {
            reload_widget_settings,
        },
        save,
    )
}

pub(crate) fn persist_profile_change(
    profile: &mut WidgetProfile,
    previous: WidgetProfile,
    save: impl FnOnce(&WidgetProfile) -> io::Result<()>,
) -> CatalogActionOutcome {
    persist_profile_change_with_commit(
        profile,
        previous,
        CatalogCommit {
            reload_widget_settings: false,
        },
        save,
    )
}

fn persist_profile_change_with_commit(
    profile: &mut WidgetProfile,
    previous: WidgetProfile,
    commit: CatalogCommit,
    save: impl FnOnce(&WidgetProfile) -> io::Result<()>,
) -> CatalogActionOutcome {
    let candidate = match profile.clone().validate() {
        Ok(candidate) => candidate,
        Err(error) => {
            *profile = previous;
            return CatalogActionOutcome::RolledBack {
                message: bounded_error("Invalid widget profile", error),
                category: CatalogFailureCategory::Validation,
            };
        }
    };

    match save(&candidate) {
        Ok(()) => {
            *profile = candidate;
            CatalogActionOutcome::Durable(commit)
        }
        Err(error) if settings_save_was_committed(&error) => {
            *profile = candidate;
            CatalogActionOutcome::CommittedWithWarning {
                commit,
                message: bounded_error("Saved, but durability is uncertain", error),
            }
        }
        Err(error) => {
            *profile = previous;
            CatalogActionOutcome::RolledBack {
                message: bounded_error("Could not save widgets", error),
                category: CatalogFailureCategory::Filesystem,
            }
        }
    }
}

pub fn paint_catalog(
    ui: &mut egui::Ui,
    profile: &WidgetProfile,
    message: Option<&str>,
    warframe_active: bool,
) -> Vec<CatalogAction> {
    let mut actions = Vec::new();
    let available = ui.available_width().max(200.0);

    ui.label(
        egui::RichText::new("Choose your widgets")
            .heading()
            .size(23.0)
            .color(TEXT_PRIMARY),
    );
    ui.label(meta_text(
        "Click a card to enable it, then customize it directly on the overlay.",
    ));
    ui.add_space(14.0);

    let columns = if available >= 680.0 { 2 } else { 1 };
    let gap = 12.0;
    let card_width = ((available - gap * (columns as f32 - 1.0)) / columns as f32).max(200.0);

    for category in WidgetCategory::ALL {
        if category == WidgetCategory::Warframe && !warframe_active {
            continue;
        }
        paint_category_header(ui, category);
        ui.add_space(7.0);
        let widgets = BUILTIN_WIDGETS
            .iter()
            .filter(|descriptor| descriptor.category == category);
        if columns == 1 {
            for descriptor in widgets {
                paint_widget_card(ui, profile, descriptor, card_width, &mut actions);
                ui.add_space(gap);
            }
        } else {
            egui::Grid::new(("widget-catalog-category", category.label()))
                .num_columns(columns)
                .spacing(egui::vec2(gap, gap))
                .show(ui, |ui| {
                    let mut column = 0;
                    for descriptor in widgets {
                        ui.vertical(|ui| {
                            paint_widget_card(ui, profile, descriptor, card_width, &mut actions);
                        });
                        column += 1;
                        if column == columns {
                            ui.end_row();
                            column = 0;
                        }
                    }
                    if column != 0 {
                        ui.end_row();
                    }
                });
        }
        ui.add_space(18.0);
    }

    if let Some(message) = message {
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(251, 113, 133, 22))
            .stroke(Stroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(251, 113, 133, 90),
            ))
            .corner_radius(9)
            .inner_margin(egui::Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.colored_label(Color32::from_rgb(251, 113, 133), message);
            });
    }

    actions
}

pub(crate) fn paint_gated_options(
    ui: &mut egui::Ui,
    available: bool,
    unavailable_message: &str,
    paint: impl FnOnce(&mut egui::Ui),
) {
    if !available {
        ui.colored_label(TEXT_MUTED, unavailable_message);
    }
    ui.add_enabled_ui(available, paint);
}

fn paint_category_header(ui: &mut egui::Ui, category: WidgetCategory) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 30.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, ACCENT);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(title_text(category.label()));
            ui.label(meta_text(category.description()));
        });
    });
}

fn paint_widget_card(
    ui: &mut egui::Ui,
    profile: &WidgetProfile,
    descriptor: &WidgetDescriptor,
    width: f32,
    actions: &mut Vec<CatalogAction>,
) {
    let settings = profile.settings(descriptor.id);
    let fill = if settings.enabled {
        Color32::from_rgba_unmultiplied(23, 27, 20, 238)
    } else {
        SURFACE_RAISED
    };
    let stroke = if settings.enabled {
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(163, 230, 53, 80))
    } else {
        Stroke::new(1.0, PANEL_STROKE)
    };

    let card = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(12)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(width - 26.0);
            ui.horizontal(|ui| {
                paint_widget_glyph(ui, descriptor.glyph, 32.0, settings.enabled);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(
                        egui::RichText::new(descriptor.name)
                            .size(14.0)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(descriptor.description)
                                .size(11.0)
                                .color(TEXT_MUTED),
                        )
                        .wrap(),
                    );
                });
            });
        });
    let label = if settings.enabled {
        format!("Disable {}", descriptor.name)
    } else {
        format!("Enable {}", descriptor.name)
    };
    let response = ui
        .interact(
            card.response.rect,
            ui.id().with(("widget-catalog-card", descriptor.name)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    if response.hovered() {
        ui.painter().rect_stroke(
            card.response.rect,
            12.0,
            Stroke::new(1.0, ACCENT.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    }
    if response.clicked() {
        actions.push(CatalogAction::SetEnabled(descriptor.id, !settings.enabled));
    }
}

pub(crate) fn paint_profile_options(
    ui: &mut egui::Ui,
    profile: &WidgetProfile,
    id: WidgetId,
    actions: &mut Vec<CatalogAction>,
) {
    match id {
        WidgetId::Clock => {
            let mut visible = profile.clock_display.show_date;
            if ui.checkbox(&mut visible, "Show date").changed() {
                actions.push(CatalogAction::SetClockDateVisible(visible));
            }
        }
        WidgetId::Performance => {
            ui.label(eyebrow_text("LAYOUT"));
            let mut layout = profile.performance_display.layout;
            let horizontal_changed = ui
                .radio_value(&mut layout, PerformanceLayout::Horizontal, "Horizontal")
                .changed();
            let vertical_changed = ui
                .radio_value(&mut layout, PerformanceLayout::Vertical, "Vertical")
                .changed();
            if horizontal_changed || vertical_changed {
                actions.push(CatalogAction::SetPerformanceLayout(layout));
            }
        }
        WidgetId::Notes => {
            let display = profile.notes_display;
            let mut show_note = display.show_note;
            if ui
                .add_enabled(
                    !show_note || display.show_checklist,
                    egui::Checkbox::new(&mut show_note, "Show note"),
                )
                .on_disabled_hover_text("Keep at least one section visible")
                .changed()
            {
                actions.push(CatalogAction::SetNotesNoteVisible(show_note));
            }
            let mut show_checklist = display.show_checklist;
            if ui
                .add_enabled(
                    !show_checklist || display.show_note,
                    egui::Checkbox::new(&mut show_checklist, "Show checklist"),
                )
                .on_disabled_hover_text("Keep at least one section visible")
                .changed()
            {
                actions.push(CatalogAction::SetNotesChecklistVisible(show_checklist));
            }
        }
        WidgetId::DiscordVoice => {
            ui.label(eyebrow_text("ALIGNMENT"));
            let mut alignment = profile.discord_voice_display.alignment;
            let left_changed = ui
                .radio_value(&mut alignment, DiscordVoiceAlignment::Left, "Left")
                .changed();
            let right_changed = ui
                .radio_value(&mut alignment, DiscordVoiceAlignment::Right, "Right")
                .changed();
            if left_changed || right_changed {
                actions.push(CatalogAction::SetDiscordAlignment(alignment));
            }

            ui.label(eyebrow_text("PARTICIPANTS"));
            let mut limit = profile.discord_voice_display.participant_limit;
            let response = ui.add(
                egui::Slider::new(
                    &mut limit,
                    DISCORD_PARTICIPANT_LIMIT_MIN..=DISCORD_PARTICIPANT_LIMIT_MAX,
                )
                .text("Visible people"),
            );
            if (response.changed() && !response.dragged()) || response.drag_stopped() {
                actions.push(CatalogAction::SetDiscordParticipantLimit(limit));
            }
        }
        _ => {}
    }
}

fn bounded_error(prefix: &str, error: impl fmt::Display) -> String {
    let message = format!("{prefix} : {error}");
    if message.chars().count() <= CATALOG_ERROR_MAX_CHARS {
        return message;
    }

    let mut bounded = message
        .chars()
        .take(CATALOG_ERROR_MAX_CHARS.saturating_sub(1))
        .collect::<String>();
    bounded.push('…');
    bounded
}
