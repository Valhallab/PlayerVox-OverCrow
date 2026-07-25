use eframe::egui::{self, Color32, Sense, Vec2, WidgetInfo, WidgetType, vec2};
use overcrow_config::NotesDisplaySettings;
use overcrow_protocol::OverlayMode;

use crate::notes::{NOTES_PAGE_MAX_COUNT, NotePage, NotesCommand};

use super::{
    CatalogAction,
    chrome::{
        BODY_SIZE, META_SIZE, ResizeGripOutcome, accent_error, accent_warn, apply_scale,
        fixed_panel_constraints, meta_text, options_menu, panel_frame, report_fixed_panel_size,
        resize_grip, title_text,
    },
};

mod state;

pub use state::NotesWidgetState;

#[derive(Clone, Debug, PartialEq)]
pub enum NotesWidgetAction {
    Command(NotesCommand),
    Catalog(CatalogAction),
}

pub struct NotesResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub resize: ResizeGripOutcome,
    pub actions: Vec<NotesWidgetAction>,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_notes(
    ui: &mut egui::Ui,
    current_position: egui::Pos2,
    panel_size: Vec2,
    state: &mut NotesWidgetState,
    display: NotesDisplaySettings,
    scale: f32,
    mode: OverlayMode,
    transparent_background: bool,
    draggable: bool,
    margin: f32,
) -> NotesResponse {
    let panel_size = super::chrome::clamp_panel_size(panel_size);
    let interactive = mode == OverlayMode::Interactive;
    let mut actions = Vec::new();
    let mut resize = ResizeGripOutcome::default();
    let viewport = ui.max_rect();
    // Reserve the frame's vertical margins and stroke so pathological content
    // cannot make the Area exceed the game viewport.
    let safe_height = (viewport.height() - margin * 2.0 - 28.0).max(1.0);

    let response = egui::Area::new(egui::Id::new("notes-panel"))
        .current_pos(current_position)
        .movable(draggable)
        .interactable(true)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            apply_scale(ui, scale);
            panel_frame(transparent_background).show(ui, |ui| {
                fixed_panel_constraints(ui, panel_size, mode, safe_height);
                paint_header(ui, state, display, interactive, &mut actions);
                ui.add_space(4.0);

                if !state.ready() {
                    ui.label(meta_text("Loading notes…"));
                } else {
                    if display.show_note {
                        paint_tabs(ui, state, interactive, &mut actions);
                        ui.add_space(6.0);
                    }
                    paint_active_note_region(
                        ui,
                        state,
                        display,
                        interactive,
                        panel_size,
                        &mut actions,
                    );
                }

                let panel_rect = ui.min_rect();
                resize = resize_grip(ui, panel_rect, interactive);
            });
        });

    let measured = response.response.rect.size().max(vec2(1.0, 1.0));
    NotesResponse {
        size: report_fixed_panel_size(panel_size, measured, mode),
        position: response.response.rect.min,
        dragged: response.response.dragged() && !resize.dragging,
        drag_stopped: response.response.drag_stopped() && !resize.dragging && !resize.drag_stopped,
        resize,
        actions,
    }
}

fn paint_active_note_region(
    ui: &mut egui::Ui,
    state: &mut NotesWidgetState,
    display: NotesDisplaySettings,
    interactive: bool,
    panel_size: Vec2,
    actions: &mut Vec<NotesWidgetAction>,
) {
    let max_height = ui.available_height().max(1.0);
    egui::ScrollArea::vertical()
        .id_salt(("notes-content", interactive))
        .max_height(max_height)
        .auto_shrink([false, !interactive])
        .show(ui, |ui| {
            paint_active_note(ui, state, display, interactive, panel_size, actions);
        });
}

fn paint_header(
    ui: &mut egui::Ui,
    state: &NotesWidgetState,
    display: NotesDisplaySettings,
    interactive: bool,
    actions: &mut Vec<NotesWidgetAction>,
) {
    ui.horizontal(|ui| {
        ui.label(title_text("NOTES"));
        if state.save_pending() {
            ui.label(meta_text("Saving…"));
        } else if state.durability_warning() {
            ui.colored_label(
                accent_warn(),
                egui::RichText::new("Saved with warning").small(),
            );
        }
        if interactive {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                options_menu(ui, |ui| {
                    let mut show_note = display.show_note;
                    if ui
                        .add_enabled(
                            !show_note || display.show_checklist,
                            egui::Checkbox::new(&mut show_note, "Show note"),
                        )
                        .on_disabled_hover_text("Keep at least one section visible")
                        .changed()
                    {
                        actions.push(NotesWidgetAction::Catalog(
                            CatalogAction::SetNotesNoteVisible(show_note),
                        ));
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
                        actions.push(NotesWidgetAction::Catalog(
                            CatalogAction::SetNotesChecklistVisible(show_checklist),
                        ));
                    }
                });
            });
        }
    });
    if let Some(message) = state.message() {
        ui.colored_label(accent_error(), egui::RichText::new(message).size(META_SIZE));
    }
}

fn paint_tabs(
    ui: &mut egui::Ui,
    state: &NotesWidgetState,
    interactive: bool,
    actions: &mut Vec<NotesWidgetAction>,
) {
    if interactive {
        ui.horizontal(|ui| {
            let tabs_width = (ui.available_width() - 32.0).max(80.0);
            egui::ScrollArea::horizontal()
                .id_salt("notes-tabs")
                .max_width(tabs_width)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for note in &state.document().notes {
                            let dirty = state.draft_is_dirty(&note.id);
                            let label = if dirty {
                                format!("{} •", note.title)
                            } else {
                                note.title.clone()
                            };
                            if ui
                                .selectable_label(state.document().active_note_id == note.id, label)
                                .clicked()
                                && state.document().active_note_id != note.id
                            {
                                actions.push(NotesWidgetAction::Command(
                                    NotesCommand::SetActiveNote {
                                        id: note.id.clone(),
                                    },
                                ));
                            }
                        }
                    });
                });
            if ui
                .add_enabled(
                    state.document().notes.len() < NOTES_PAGE_MAX_COUNT,
                    egui::Button::new("+"),
                )
                .on_hover_text("Add note")
                .clicked()
            {
                actions.push(NotesWidgetAction::Command(NotesCommand::AddNote {
                    title: format!("Note {}", state.document().next_note_id),
                }));
            }
        });
    } else {
        if let Some(note) = state.document().active_note() {
            ui.label(
                egui::RichText::new(&note.title)
                    .size(BODY_SIZE + 1.0)
                    .strong()
                    .color(Color32::from_gray(230)),
            );
        }
    }
}

fn paint_active_note(
    ui: &mut egui::Ui,
    state: &mut NotesWidgetState,
    display: NotesDisplaySettings,
    interactive: bool,
    panel_size: Vec2,
    actions: &mut Vec<NotesWidgetAction>,
) {
    let note_id = state.document().active_note_id.clone();

    if display.show_note {
        state.ensure_draft(&note_id);
    }
    if interactive && display.show_note {
        paint_note_editor(ui, state, &note_id, actions);
    } else if display.show_note
        && let Some(note) = state.document().active_note()
    {
        paint_note_body(ui, note);
    }

    if display.show_note && display.show_checklist {
        ui.add_space(8.0);
    }
    if display.show_checklist {
        paint_checklist(ui, state, &note_id, interactive, panel_size, actions);
    }
}

fn paint_note_editor(
    ui: &mut egui::Ui,
    state: &mut NotesWidgetState,
    note_id: &str,
    actions: &mut Vec<NotesWidgetAction>,
) {
    ui.label(meta_text("TITLE"));
    if let Some(draft) = state.drafts.get_mut(note_id) {
        ui.add(
            egui::TextEdit::singleline(&mut draft.title)
                .desired_width(f32::INFINITY)
                .hint_text("Note title"),
        );
        ui.add_space(4.0);
        ui.label(meta_text("NOTE"));
        ui.add(
            egui::TextEdit::multiline(&mut draft.body)
                .desired_width(f32::INFINITY)
                .desired_rows(5)
                .hint_text("Write anything…"),
        );
    }

    ui.horizontal(|ui| {
        let dirty = state.draft_is_dirty(note_id);
        let valid_title = state
            .drafts
            .get(note_id)
            .is_some_and(|draft| !draft.title.as_str().trim().is_empty());
        if ui
            .add_enabled(dirty && valid_title, egui::Button::new("Save"))
            .clicked()
            && let Some(draft) = state.drafts.get(note_id)
        {
            actions.push(NotesWidgetAction::Command(NotesCommand::UpdateNote {
                id: note_id.to_owned(),
                title: draft.title.as_str().trim().to_owned(),
                body: draft.body.as_str().to_owned(),
            }));
        }
        if dirty {
            ui.label(meta_text("Unsaved"));
        }
        paint_delete_note(ui, state, note_id, actions);
    });
}

fn paint_delete_note(
    ui: &mut egui::Ui,
    state: &mut NotesWidgetState,
    note_id: &str,
    actions: &mut Vec<NotesWidgetAction>,
) {
    if state.document().notes.len() <= 1 {
        return;
    }
    if state.delete_confirmation.as_deref() == Some(note_id) {
        if ui.small_button("Delete note?").clicked() {
            actions.push(NotesWidgetAction::Command(NotesCommand::RemoveNote {
                id: note_id.to_owned(),
            }));
        }
        if ui.small_button("Cancel").clicked() {
            state.delete_confirmation = None;
        }
    } else if ui.small_button("Delete").clicked() {
        state.delete_confirmation = Some(note_id.to_owned());
    }
}

fn paint_note_body(ui: &mut egui::Ui, note: &NotePage) {
    ui.label(meta_text("NOTE"));
    if note.body.is_empty() {
        ui.label(meta_text("Nothing written yet"));
    } else {
        ui.label(
            egui::RichText::new(&note.body)
                .size(BODY_SIZE)
                .color(Color32::from_gray(225)),
        );
    }
}

fn paint_checklist(
    ui: &mut egui::Ui,
    state: &mut NotesWidgetState,
    note_id: &str,
    interactive: bool,
    panel_size: Vec2,
    actions: &mut Vec<NotesWidgetAction>,
) {
    ui.label(meta_text("CHECKLIST"));
    if interactive {
        let draft = state.new_item_draft_mut(note_id);
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(draft)
                    .desired_width((panel_size.x - 96.0).max(80.0))
                    .hint_text("Add something…"),
            );
            let submitted =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let text = draft.as_str().trim();
            if (ui
                .add_enabled(!text.is_empty(), egui::Button::new("Add"))
                .clicked()
                || submitted)
                && !text.is_empty()
            {
                actions.push(NotesWidgetAction::Command(NotesCommand::AddItem {
                    note_id: note_id.to_owned(),
                    text: text.to_owned(),
                }));
            }
        });
    }

    let note = state.document().note(note_id).expect("active note exists");
    if note.items.is_empty() {
        ui.label(meta_text("Nothing here yet"));
        return;
    }
    for item in checklist_display_order(note) {
        ui.horizontal(|ui| {
            let toggled = paint_check_control(ui, item.checked, interactive, &item.text);
            if toggled {
                actions.push(NotesWidgetAction::Command(NotesCommand::SetChecked {
                    note_id: note_id.to_owned(),
                    id: item.id.clone(),
                    checked: !item.checked,
                }));
            }
            let mut text = egui::RichText::new(&item.text)
                .size(BODY_SIZE)
                .color(if item.checked {
                    Color32::from_gray(145)
                } else {
                    Color32::from_gray(225)
                });
            if item.checked {
                text = text.strikethrough();
            }
            ui.label(text);
            if interactive && ui.small_button("×").on_hover_text("Remove entry").clicked() {
                actions.push(NotesWidgetAction::Command(NotesCommand::RemoveItem {
                    note_id: note_id.to_owned(),
                    id: item.id.clone(),
                }));
            }
        });
    }
}

pub(crate) fn checklist_display_order(note: &NotePage) -> Vec<&crate::notes::ChecklistItem> {
    note.items
        .iter()
        .filter(|item| !item.checked)
        .chain(note.items.iter().filter(|item| item.checked))
        .collect()
}

fn paint_check_control(ui: &mut egui::Ui, checked: bool, interactive: bool, label: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        vec2(18.0, 18.0),
        if interactive {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let color = if checked {
        Color32::from_rgb(150, 220, 120)
    } else {
        Color32::from_gray(150)
    };
    ui.painter()
        .circle_stroke(rect.center(), 6.0, egui::Stroke::new(1.5, color));
    if checked {
        let center = rect.center();
        ui.painter().line_segment(
            [center + vec2(-3.0, 0.0), center + vec2(-0.8, 2.4)],
            egui::Stroke::new(1.7, color),
        );
        ui.painter().line_segment(
            [center + vec2(-0.8, 2.4), center + vec2(3.8, -3.0)],
            egui::Stroke::new(1.7, color),
        );
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, interactive, checked, label));
    if interactive {
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    } else {
        false
    }
}

pub fn notes_action_allowed(mode: OverlayMode, _command: &NotesCommand) -> bool {
    mode == OverlayMode::Interactive
}
