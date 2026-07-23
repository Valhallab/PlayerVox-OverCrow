use std::{fmt, io};

use eframe::egui;
use overcrow_config::{WidgetId, WidgetProfile, settings_save_was_committed};
use overcrow_protocol::OverlayMode;

use super::BUILTIN_WIDGETS;

pub const CATALOG_ERROR_MAX_CHARS: usize = 180;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CatalogAction {
    SetEnabled(WidgetId, bool),
    SetPassive(WidgetId, bool),
    SetTransparentBackground(WidgetId, bool),
    ResetPosition(WidgetId),
}

impl CatalogAction {
    pub const fn widget_id(self) -> WidgetId {
        match self {
            Self::SetEnabled(id, _)
            | Self::SetPassive(id, _)
            | Self::SetTransparentBackground(id, _)
            | Self::ResetPosition(id) => id,
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
    let mut candidate = profile.clone();
    let manual_enabled_before = candidate.manual_stopwatch.enabled;

    match action {
        CatalogAction::SetEnabled(id, enabled) => candidate.settings_mut(id).enabled = enabled,
        CatalogAction::SetPassive(id, visible) => {
            candidate.settings_mut(id).show_in_passive = visible;
        }
        CatalogAction::SetTransparentBackground(id, transparent) => {
            candidate.settings_mut(id).transparent_background = transparent;
        }
        CatalogAction::ResetPosition(id) => {
            candidate.settings_mut(id).position = id.default_position();
        }
    }

    let candidate = match candidate.validate() {
        Ok(candidate) => candidate,
        Err(error) => {
            return CatalogActionOutcome::RolledBack {
                message: bounded_error("Invalid widget profile", error),
                category: CatalogFailureCategory::Validation,
            };
        }
    };

    let reload_widget_settings = matches!(
        action,
        CatalogAction::SetEnabled(WidgetId::ManualStopwatch, _)
    ) && candidate.manual_stopwatch.enabled != manual_enabled_before;
    let commit = CatalogCommit {
        reload_widget_settings,
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
        Err(error) => CatalogActionOutcome::RolledBack {
            message: bounded_error("Could not save widgets", error),
            category: CatalogFailureCategory::Filesystem,
        },
    }
}

pub fn paint_catalog(
    ui: &mut egui::Ui,
    profile: &WidgetProfile,
    message: Option<&str>,
) -> Vec<CatalogAction> {
    let mut actions = Vec::new();
    ui.set_min_width(520.0);
    ui.heading("Widgets");
    ui.add_space(4.0);

    for descriptor in BUILTIN_WIDGETS {
        let settings = profile.settings(descriptor.id);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_min_width(230.0);
                ui.label(egui::RichText::new(descriptor.name).strong());
                ui.label(
                    egui::RichText::new(descriptor.description)
                        .small()
                        .color(egui::Color32::from_gray(170)),
                );
                if matches!(
                    descriptor.id,
                    WidgetId::WarframeStatus
                        | WidgetId::WarframeFissures
                        | WidgetId::WarframeMarket
                        | WidgetId::WarframeSortie
                        | WidgetId::WarframeInvasions
                ) {
                    ui.label(
                        egui::RichText::new("Warframe only · resizable (bottom-right corner).")
                            .small()
                            .color(egui::Color32::from_gray(140)),
                    );
                }
            });

            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let mut enabled = settings.enabled;
                    if ui.checkbox(&mut enabled, "Enabled").changed() {
                        actions.push(CatalogAction::SetEnabled(descriptor.id, enabled));
                    }

                    let mut passive = settings.show_in_passive;
                    if ui.checkbox(&mut passive, "Passive mode").changed() {
                        actions.push(CatalogAction::SetPassive(descriptor.id, passive));
                    }

                    if ui.small_button("Reset").clicked() {
                        actions.push(CatalogAction::ResetPosition(descriptor.id));
                    }
                });
                let mut transparent = settings.transparent_background;
                if ui
                    .checkbox(&mut transparent, "Transparent background")
                    .changed()
                {
                    actions.push(CatalogAction::SetTransparentBackground(
                        descriptor.id,
                        transparent,
                    ));
                }
            });
        });
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
    }

    if let Some(message) = message {
        ui.colored_label(egui::Color32::from_rgb(255, 150, 150), message);
    }

    actions
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
