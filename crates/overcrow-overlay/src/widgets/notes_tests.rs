use std::cell::Cell;

use eframe::egui::{
    self, Event, RawInput, Rect,
    accesskit::{Action as AccessKitAction, ActionRequest, Role, TreeId},
    pos2, vec2,
};
use overcrow_config::NotesDisplaySettings;
use overcrow_protocol::OverlayMode;

use crate::notes::{NOTES_NOTE_MAX_BYTES, NotesCommand, NotesDocument, NotesUpdate};

use super::{
    NotesWidgetState,
    notes::{checklist_display_order, paint_notes},
};

fn update(document: NotesDocument, save_pending: bool, error: Option<&str>) -> NotesUpdate {
    NotesUpdate {
        document,
        save_pending,
        error: error.map(str::to_owned),
        durability_warning: false,
    }
}

fn update_general(body: impl Into<String>) -> NotesCommand {
    NotesCommand::UpdateNote {
        id: "note-1".to_owned(),
        title: "General".to_owned(),
        body: body.into(),
    }
}

#[test]
fn provider_load_initializes_the_visible_document_and_note_draft() {
    let mut document = NotesDocument::default();
    document.set_note("saved note").unwrap();
    let mut state = NotesWidgetState::default();

    state.apply_update(update(document.clone(), false, None));

    assert!(state.ready());
    assert_eq!(state.document(), &document);
    assert_eq!(state.note_draft(), "saved note");
}

#[test]
fn accepted_commands_stay_visible_while_the_repository_save_is_pending() {
    let mut state = NotesWidgetState::default();
    state.apply_update(update(NotesDocument::default(), false, None));
    let command = NotesCommand::AddItem {
        note_id: "note-1".to_owned(),
        text: "Do the thing".to_owned(),
    };

    state.accept(&command).unwrap();
    state.apply_update(update(NotesDocument::default(), true, None));

    let active = state
        .document()
        .active_note()
        .expect("valid test document has an active note");
    assert_eq!(active.items.len(), 1);
    assert_eq!(active.items[0].text, "Do the thing");
    assert!(state.save_pending());
}

#[test]
fn settled_save_failure_keeps_the_submitted_note_as_a_dirty_draft() {
    let committed = NotesDocument::default();
    let mut state = NotesWidgetState::default();
    state.apply_update(update(committed.clone(), false, None));
    state.accept(&update_general("unsaved")).unwrap();

    state.apply_update(update(
        committed.clone(),
        false,
        Some("notes repository failed"),
    ));

    assert_eq!(state.document(), &committed);
    assert_eq!(state.note_draft(), "unsaved");
    assert!(state.note_is_dirty());
    assert_eq!(state.message(), Some("notes repository failed"));
}

#[test]
fn settled_save_failure_restores_the_submitted_checklist_input() {
    let committed = NotesDocument::default();
    let mut state = NotesWidgetState::default();
    state.apply_update(update(committed.clone(), false, None));
    state.set_new_item_draft_for_tests("note-1", "Retry this entry");
    state
        .accept(&NotesCommand::AddItem {
            note_id: "note-1".to_owned(),
            text: "Retry this entry".to_owned(),
        })
        .unwrap();

    state.apply_update(update(
        committed.clone(),
        false,
        Some("notes repository failed"),
    ));

    assert_eq!(state.document(), &committed);
    assert_eq!(
        state.new_item_draft_for_tests("note-1"),
        Some("Retry this entry")
    );
}

#[test]
fn provider_updates_do_not_replace_an_unsaved_note_draft() {
    let mut state = NotesWidgetState::default();
    state.apply_update(update(NotesDocument::default(), false, None));
    state.set_note_draft("local draft");
    let mut committed = NotesDocument::default();
    committed.add_item("saved item").unwrap();

    state.apply_update(update(committed.clone(), false, None));

    assert_eq!(state.document(), &committed);
    assert_eq!(state.note_draft(), "local draft");
    assert!(state.note_is_dirty());
}

#[test]
fn switching_notes_preserves_an_unsaved_draft() {
    let mut document = NotesDocument::default();
    let second = document.add_note("Second").unwrap();
    document.set_active_note("note-1").unwrap();
    let mut state = NotesWidgetState::default();
    state.apply_update(update(document, false, None));
    state.set_note_draft("draft on General");

    state
        .accept(&NotesCommand::SetActiveNote { id: second })
        .unwrap();
    state
        .accept(&NotesCommand::SetActiveNote {
            id: "note-1".to_owned(),
        })
        .unwrap();

    assert_eq!(state.note_draft(), "draft on General");
    assert!(state.note_is_dirty());
}

#[test]
fn checked_entries_render_after_open_entries_without_reordering_each_group() {
    let mut document = NotesDocument::default();
    let first = document.add_item("done first").unwrap();
    document.add_item("open first").unwrap();
    let third = document.add_item("done second").unwrap();
    document.add_item("open second").unwrap();
    document.set_checked(&first, true).unwrap();
    document.set_checked(&third, true).unwrap();

    let ordered = checklist_display_order(
        document
            .active_note()
            .expect("valid test document has an active note"),
    )
    .into_iter()
    .map(|item| item.text.as_str())
    .collect::<Vec<_>>();

    assert_eq!(
        ordered,
        ["open first", "open second", "done first", "done second"]
    );
}

#[test]
fn note_drafts_are_bounded_on_a_utf8_boundary_before_storage() {
    let mut state = NotesWidgetState::default();

    state.set_note_draft(&"é".repeat(NOTES_NOTE_MAX_BYTES));

    assert!(state.note_draft().len() <= NOTES_NOTE_MAX_BYTES);
    assert!(
        state
            .note_draft()
            .is_char_boundary(state.note_draft().len())
    );
}

#[test]
fn immediate_command_errors_do_not_hide_an_existing_pending_save() {
    let mut state = NotesWidgetState::default();
    state.apply_update(update(NotesDocument::default(), false, None));
    state
        .accept(&NotesCommand::AddItem {
            note_id: "note-1".to_owned(),
            text: "pending".to_owned(),
        })
        .unwrap();

    state.set_error("next command was rejected");

    assert!(state.save_pending());
    assert_eq!(state.message(), Some("next command was rejected"));
}

#[test]
fn passive_mode_rejects_every_mutating_command() {
    for command in [
        update_general("note"),
        NotesCommand::AddItem {
            note_id: "note-1".to_owned(),
            text: "item".to_owned(),
        },
        NotesCommand::SetChecked {
            note_id: "note-1".to_owned(),
            id: "local-1".to_owned(),
            checked: true,
        },
        NotesCommand::RemoveItem {
            note_id: "note-1".to_owned(),
            id: "local-1".to_owned(),
        },
    ] {
        assert!(!super::notes_action_allowed(OverlayMode::Passive, &command));
        assert!(super::notes_action_allowed(
            OverlayMode::Interactive,
            &command
        ));
    }
}

#[test]
fn oversized_note_paste_is_bounded_before_text_layout() {
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut state = NotesWidgetState::default();
    state.apply_update(update(NotesDocument::default(), false, None));

    let first = paint_notes_frame(&context, &mut state, Vec::new());
    let editor_id = first
        .platform_output
        .accesskit_update
        .expect("accessibility tree")
        .nodes
        .into_iter()
        .find_map(|(id, node)| (node.role() == Role::MultilineTextInput).then_some(id))
        .expect("note editor");
    let focus = Event::AccessKitActionRequest(ActionRequest {
        action: AccessKitAction::Focus,
        target_tree: TreeId::ROOT,
        target_node: editor_id,
        data: None,
    });
    let _ = paint_notes_frame(&context, &mut state, vec![focus]);

    let pasted = "🎮".repeat(NOTES_NOTE_MAX_BYTES);
    let output = paint_notes_frame(&context, &mut state, vec![Event::Paste(pasted)]);
    let editor = output
        .platform_output
        .accesskit_update
        .expect("accessibility update")
        .nodes
        .into_iter()
        .find_map(|(_, node)| (node.role() == Role::MultilineTextInput).then_some(node))
        .expect("updated note editor");

    assert!(
        editor
            .value()
            .is_some_and(|value| value.len() <= NOTES_NOTE_MAX_BYTES)
    );
    assert!(state.note_draft().len() <= NOTES_NOTE_MAX_BYTES);
}

#[test]
fn passive_height_tracks_visible_content_without_changing_the_configured_height() {
    let context = egui::Context::default();
    let mut short = NotesDocument::default();
    short.set_note("One line").unwrap();
    let mut short_state = NotesWidgetState::default();
    short_state.apply_update(update(short, false, None));
    let short_size = paint_notes_size(
        &context,
        &mut short_state,
        OverlayMode::Passive,
        NotesDisplaySettings::default(),
    );

    let mut long = NotesDocument::default();
    long.set_note(
        (0..10)
            .map(|n| format!("Line {n}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let mut long_state = NotesWidgetState::default();
    long_state.apply_update(update(long, false, None));
    let long_size = paint_notes_size(
        &context,
        &mut long_state,
        OverlayMode::Passive,
        NotesDisplaySettings::default(),
    );
    let checklist_only_size = paint_notes_size(
        &context,
        &mut long_state,
        OverlayMode::Passive,
        NotesDisplaySettings {
            show_note: false,
            show_checklist: true,
        },
    );

    assert!(short_size.y < 280.0);
    assert!(long_size.y > short_size.y);
    assert!(checklist_only_size.y < long_size.y);
}

#[test]
fn excessive_passive_content_is_bounded_to_the_game_viewport() {
    let context = egui::Context::default();
    let mut document = NotesDocument::default();
    document
        .set_note(
            (0..300)
                .map(|line| format!("Visible line {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    let mut state = NotesWidgetState::default();
    state.apply_update(update(document, false, None));

    let size = paint_notes_size(
        &context,
        &mut state,
        OverlayMode::Passive,
        NotesDisplaySettings::default(),
    );

    assert!(size.y <= 900.0, "passive panel exceeded viewport: {size:?}");
}

#[test]
fn interactive_checklist_control_exposes_checkbox_semantics() {
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut document = NotesDocument::default();
    document.add_item("Accessible entry").unwrap();
    let mut state = NotesWidgetState::default();
    state.apply_update(update(document, false, None));

    let output = paint_notes_frame(&context, &mut state, Vec::new());
    let checkbox = output
        .platform_output
        .accesskit_update
        .expect("accessibility tree")
        .nodes
        .into_iter()
        .find_map(|(_, node)| (node.role() == Role::CheckBox).then_some(node))
        .expect("checklist checkbox");

    assert_eq!(checkbox.label(), Some("Accessible entry"));
}

#[test]
fn hiding_note_removes_tabs_title_editor_and_body_as_one_block() {
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut document = NotesDocument::default();
    document.set_note("Private body").unwrap();
    document.add_note("Second tab").unwrap();
    document.set_active_note("note-1").unwrap();
    let mut state = NotesWidgetState::default();
    state.apply_update(update(document, false, None));

    let output = paint_notes_frame_with_display(
        &context,
        &mut state,
        Vec::new(),
        NotesDisplaySettings {
            show_note: false,
            show_checklist: true,
        },
    );
    let text = accessible_text(&output);

    assert!(!text.iter().any(|value| value == "General"));
    assert!(!text.iter().any(|value| value == "Second tab"));
    assert!(!text.iter().any(|value| value == "TITLE"));
    assert!(!text.iter().any(|value| value == "NOTE"));
    assert!(!text.iter().any(|value| value == "Private body"));
    assert!(text.iter().any(|value| value == "CHECKLIST"));
}

#[test]
fn hiding_checklist_removes_heading_input_and_entries_as_one_block() {
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut document = NotesDocument::default();
    document.set_note("Visible body").unwrap();
    document.add_item("Hidden entry").unwrap();
    let mut state = NotesWidgetState::default();
    state.apply_update(update(document, false, None));

    let output = paint_notes_frame_with_display(
        &context,
        &mut state,
        Vec::new(),
        NotesDisplaySettings {
            show_note: true,
            show_checklist: false,
        },
    );
    let text = accessible_text(&output);

    assert!(text.iter().any(|value| value == "General"));
    assert!(text.iter().any(|value| value == "TITLE"));
    assert!(!text.iter().any(|value| value == "CHECKLIST"));
    assert!(!text.iter().any(|value| value == "Add something…"));
    assert!(!text.iter().any(|value| value == "Hidden entry"));
}

fn paint_notes_size(
    context: &egui::Context,
    state: &mut NotesWidgetState,
    mode: OverlayMode,
    display: NotesDisplaySettings,
) -> egui::Vec2 {
    let size = Cell::new(egui::Vec2::ZERO);
    let _ = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 900.0))),
            ..RawInput::default()
        },
        |ui| {
            size.set(
                paint_notes(
                    ui,
                    pos2(8.0, 8.0),
                    vec2(360.0, 280.0),
                    state,
                    display,
                    1.0,
                    mode,
                    false,
                    false,
                    0.0,
                )
                .size,
            );
        },
    );
    size.get()
}

fn paint_notes_frame(
    context: &egui::Context,
    state: &mut NotesWidgetState,
    events: Vec<Event>,
) -> egui::FullOutput {
    paint_notes_frame_with_display(context, state, events, NotesDisplaySettings::default())
}

fn paint_notes_frame_with_display(
    context: &egui::Context,
    state: &mut NotesWidgetState,
    events: Vec<Event>,
    display: NotesDisplaySettings,
) -> egui::FullOutput {
    context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            events,
            ..RawInput::default()
        },
        |ui| {
            paint_notes(
                ui,
                pos2(8.0, 8.0),
                vec2(360.0, 280.0),
                state,
                display,
                1.0,
                OverlayMode::Interactive,
                false,
                false,
                0.0,
            );
        },
    )
}

fn accessible_text(output: &egui::FullOutput) -> Vec<String> {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accessibility tree")
        .nodes
        .iter()
        .flat_map(|(_, node)| {
            [
                node.label().map(str::to_owned),
                node.value().map(str::to_owned),
            ]
        })
        .flatten()
        .collect()
}
