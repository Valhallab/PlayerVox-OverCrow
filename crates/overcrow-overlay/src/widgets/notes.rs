use eframe::egui::{self, Sense, Vec2, WidgetInfo, WidgetType, vec2};
use overcrow_config::NotesDisplaySettings;
use overcrow_protocol::OverlayMode;

use crate::notes::{NOTES_PAGE_MAX_COUNT, NotePage, NotesCommand};

use super::chrome::{
    ACCENT, BODY_SIZE, ControlIcon, META_SIZE, ResizeGripOutcome, TEXT_MUTED, TEXT_PRIMARY,
    accent_error, accent_warn, apply_scale, compact_icon_button, elevated_frame, eyebrow_text,
    fixed_panel_constraints, icon_button, meta_text, panel_frame, primary_button, resize_grip,
    scaled_content_font_size, singleline_text_edit, standard_button, status_pill, tab_button,
};

mod state;

pub use state::NotesWidgetState;

pub struct NotesResponse {
    pub size: egui::Vec2,
    pub position: egui::Pos2,
    pub dragged: bool,
    pub drag_stopped: bool,
    pub resize: ResizeGripOutcome,
    pub actions: Vec<NotesCommand>,
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
    input_enabled: bool,
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
        .interactable(input_enabled)
        .constrain_to(viewport.shrink(margin))
        .show(ui.ctx(), |ui| {
            if !input_enabled {
                ui.disable();
            }
            apply_scale(ui, scale);
            let frame = panel_frame(transparent_background).show(ui, |ui| {
                fixed_panel_constraints(ui, panel_size, mode, safe_height, transparent_background);
                if paint_status(ui, state) {
                    ui.add_space(4.0);
                }

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
                        transparent_background,
                        panel_size,
                        &mut actions,
                    );
                }
            });
            resize = resize_grip(ui, frame.response.rect, input_enabled && interactive);
        });

    let measured = response.response.rect.size().max(vec2(1.0, 1.0));
    NotesResponse {
        size: measured,
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
    transparent_background: bool,
    panel_size: Vec2,
    actions: &mut Vec<NotesCommand>,
) {
    let max_height = ui.available_height().max(1.0);
    egui::ScrollArea::vertical()
        .id_salt(("notes-content", interactive))
        .max_height(max_height)
        .auto_shrink([false, !interactive])
        .show(ui, |ui| {
            paint_active_note(
                ui,
                state,
                display,
                interactive,
                transparent_background,
                panel_size,
                actions,
            );
        });
}

fn paint_status(ui: &mut egui::Ui, state: &NotesWidgetState) -> bool {
    let has_status = state.save_pending() || state.durability_warning();
    if has_status {
        ui.horizontal(|ui| {
            if state.save_pending() {
                status_pill(ui, "SAVING", TEXT_MUTED);
            } else if state.durability_warning() {
                status_pill(ui, "SAVED WITH WARNING", accent_warn());
            }
        });
    }
    let has_message = state.message().is_some();
    if let Some(message) = state.message() {
        ui.colored_label(
            accent_error(),
            egui::RichText::new(message).size(scaled_content_font_size(ui, META_SIZE)),
        );
    }
    has_status || has_message
}

fn paint_tabs(
    ui: &mut egui::Ui,
    state: &NotesWidgetState,
    interactive: bool,
    actions: &mut Vec<NotesCommand>,
) {
    if interactive {
        egui::ScrollArea::horizontal()
            .id_salt("notes-tabs")
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
                            .add(tab_button(
                                label,
                                state.document().active_note_id == note.id,
                            ))
                            .clicked()
                            && state.document().active_note_id != note.id
                        {
                            actions.push(NotesCommand::SetActiveNote {
                                id: note.id.clone(),
                            });
                        }
                    }
                    let can_add = state.document().notes.len() < NOTES_PAGE_MAX_COUNT;
                    let add = ui
                        .add_enabled_ui(can_add, |ui| icon_button(ui, ControlIcon::Add, "Add note"))
                        .inner;
                    if add.clicked() {
                        actions.push(NotesCommand::AddNote {
                            title: format!("Note {}", state.document().next_note_id),
                        });
                    }
                });
            });
    } else {
        if let Some(note) = state.document().active_note() {
            ui.label(
                egui::RichText::new(&note.title)
                    .size(scaled_content_font_size(ui, BODY_SIZE + 1.0))
                    .strong()
                    .color(TEXT_PRIMARY),
            );
        }
    }
}

fn paint_active_note(
    ui: &mut egui::Ui,
    state: &mut NotesWidgetState,
    display: NotesDisplaySettings,
    interactive: bool,
    transparent_background: bool,
    panel_size: Vec2,
    actions: &mut Vec<NotesCommand>,
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
        paint_note_body(ui, note, transparent_background);
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
    actions: &mut Vec<NotesCommand>,
) {
    ui.label(eyebrow_text("TITLE"));
    if let Some(draft) = state.drafts.get_mut(note_id) {
        ui.add(
            singleline_text_edit(&mut draft.title)
                .desired_width(f32::INFINITY)
                .hint_text("Note title"),
        );
        ui.add_space(4.0);
        ui.label(eyebrow_text("NOTE"));
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
            .add_enabled(dirty && valid_title, primary_button("Save changes"))
            .clicked()
            && let Some(draft) = state.drafts.get(note_id)
        {
            actions.push(NotesCommand::UpdateNote {
                id: note_id.to_owned(),
                title: draft.title.as_str().trim().to_owned(),
                body: draft.body.as_str().to_owned(),
            });
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
    actions: &mut Vec<NotesCommand>,
) {
    if state.document().notes.len() <= 1 {
        return;
    }
    if state.delete_confirmation.as_deref() == Some(note_id) {
        if ui.add(standard_button("Delete note?")).clicked() {
            actions.push(NotesCommand::RemoveNote {
                id: note_id.to_owned(),
            });
        }
        if ui.add(standard_button("Cancel")).clicked() {
            state.delete_confirmation = None;
        }
    } else if ui.add(standard_button("Delete")).clicked() {
        state.delete_confirmation = Some(note_id.to_owned());
    }
}

fn paint_note_body(ui: &mut egui::Ui, note: &NotePage, transparent_background: bool) {
    ui.label(eyebrow_text("NOTE"));
    elevated_frame(transparent_background).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        if note.body.is_empty() {
            ui.label(meta_text("Nothing written yet"));
        } else {
            ui.label(
                egui::RichText::new(&note.body)
                    .size(scaled_content_font_size(ui, BODY_SIZE))
                    .color(TEXT_PRIMARY),
            );
        }
    });
}

fn paint_checklist(
    ui: &mut egui::Ui,
    state: &mut NotesWidgetState,
    note_id: &str,
    interactive: bool,
    panel_size: Vec2,
    actions: &mut Vec<NotesCommand>,
) {
    ui.label(eyebrow_text("CHECKLIST"));
    if interactive {
        let draft = state.new_item_draft_mut(note_id);
        ui.horizontal(|ui| {
            let response = ui.add(
                singleline_text_edit(draft)
                    .desired_width((panel_size.x - 96.0).max(80.0))
                    .hint_text("Add something…"),
            );
            let submitted =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let text = draft.as_str().trim();
            if (ui
                .add_enabled(!text.is_empty(), primary_button("Add"))
                .clicked()
                || submitted)
                && !text.is_empty()
            {
                actions.push(NotesCommand::AddItem {
                    note_id: note_id.to_owned(),
                    text: text.to_owned(),
                });
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
                actions.push(NotesCommand::SetChecked {
                    note_id: note_id.to_owned(),
                    id: item.id.clone(),
                    checked: !item.checked,
                });
            }
            let mut text = egui::RichText::new(&item.text)
                .size(scaled_content_font_size(ui, BODY_SIZE))
                .color(if item.checked {
                    TEXT_MUTED
                } else {
                    TEXT_PRIMARY
                });
            if item.checked {
                text = text.strikethrough();
            }
            ui.label(text);
            if interactive && compact_icon_button(ui, ControlIcon::Remove, "Remove entry").clicked()
            {
                actions.push(NotesCommand::RemoveItem {
                    note_id: note_id.to_owned(),
                    id: item.id.clone(),
                });
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
    let color = if checked { ACCENT } else { TEXT_MUTED };
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
