use std::{any::TypeId, collections::BTreeMap, ops::Range};

use eframe::egui::{TextBuffer, text::CharIndex};

use crate::notes::{
    NOTES_ITEM_MAX_BYTES, NOTES_NOTE_MAX_BYTES, NOTES_TITLE_MAX_BYTES, NotePage, NotesCommand,
    NotesDocument, NotesError, NotesUpdate,
};

const MESSAGE_MAX_CHARS: usize = 180;

#[derive(Clone, Debug, Default)]
pub(super) struct NoteDraft {
    pub(super) title: BoundedText<NOTES_TITLE_MAX_BYTES>,
    pub(super) body: BoundedText<NOTES_NOTE_MAX_BYTES>,
}

impl NoteDraft {
    fn from_note(note: &NotePage) -> Self {
        let mut draft = Self::default();
        draft.title.set(&note.title);
        draft.body.set(&note.body);
        draft
    }

    fn differs_from(&self, note: &NotePage) -> bool {
        self.title.as_str() != note.title || self.body.as_str() != note.body
    }
}

#[derive(Clone, Debug, Default)]
pub struct NotesWidgetState {
    ready: bool,
    document: NotesDocument,
    pub(super) drafts: BTreeMap<String, NoteDraft>,
    new_item_drafts: BTreeMap<String, BoundedText<NOTES_ITEM_MAX_BYTES>>,
    pending_note_drafts: BTreeMap<String, NoteDraft>,
    pending_item_additions: BTreeMap<String, BoundedText<NOTES_ITEM_MAX_BYTES>>,
    pub(super) delete_confirmation: Option<String>,
    save_pending: bool,
    message: Option<String>,
    durability_warning: bool,
}

impl NotesWidgetState {
    pub fn apply_update(&mut self, update: NotesUpdate) {
        if !update.save_pending {
            let dirty = self.dirty_note_ids();
            self.document = update.document;
            self.reconcile_drafts(&dirty);
            if update.error.is_some() && !update.durability_warning {
                self.restore_failed_inputs(&dirty);
            }
            self.pending_note_drafts.clear();
            self.pending_item_additions.clear();
        }
        self.ready = true;
        self.save_pending = update.save_pending;
        self.message = update.error;
        self.durability_warning = update.durability_warning;
    }

    pub fn accept(&mut self, command: &NotesCommand) -> Result<(), NotesError> {
        let mut preserve = self.dirty_note_ids();
        let mut candidate = self.document.clone();
        command.apply(&mut candidate)?;
        self.document = candidate;

        match command {
            NotesCommand::UpdateNote { id, .. } => {
                preserve.retain(|dirty_id| dirty_id != id);
                if let Some(note) = self.document.note(id) {
                    let submitted = NoteDraft::from_note(note);
                    self.drafts.insert(id.clone(), submitted.clone());
                    self.pending_note_drafts.insert(id.clone(), submitted);
                }
            }
            NotesCommand::AddItem { note_id, text } => {
                self.new_item_drafts.remove(note_id);
                let mut submitted = BoundedText::default();
                submitted.set(text);
                self.pending_item_additions
                    .insert(note_id.clone(), submitted);
            }
            NotesCommand::RemoveNote { id } => {
                preserve.retain(|dirty_id| dirty_id != id);
                self.drafts.remove(id);
                self.new_item_drafts.remove(id);
                self.pending_note_drafts.remove(id);
                self.pending_item_additions.remove(id);
                if self.delete_confirmation.as_deref() == Some(id) {
                    self.delete_confirmation = None;
                }
            }
            NotesCommand::AddNote { .. } | NotesCommand::SetActiveNote { .. } => {
                self.delete_confirmation = None;
            }
            NotesCommand::SetItemText { .. }
            | NotesCommand::SetChecked { .. }
            | NotesCommand::RemoveItem { .. } => {}
        }
        self.reconcile_drafts(&preserve);
        self.save_pending = true;
        self.message = None;
        self.durability_warning = false;
        Ok(())
    }

    pub fn set_error(&mut self, error: impl ToString) {
        self.message = Some(bound_chars(error.to_string(), MESSAGE_MAX_CHARS));
        self.durability_warning = false;
    }

    pub fn ready(&self) -> bool {
        self.ready
    }

    pub fn document(&self) -> &NotesDocument {
        &self.document
    }

    #[cfg(test)]
    pub(crate) fn note_draft(&self) -> &str {
        self.active_draft().map_or("", |draft| draft.body.as_str())
    }

    #[cfg(test)]
    pub(crate) fn set_note_draft(&mut self, value: &str) {
        let note_id = self.document.active_note_id.clone();
        self.ensure_draft(&note_id);
        if let Some(draft) = self.drafts.get_mut(&note_id) {
            draft.body.set(value);
        }
    }

    #[cfg(test)]
    pub(crate) fn note_is_dirty(&self) -> bool {
        self.draft_is_dirty(&self.document.active_note_id)
    }

    pub fn save_pending(&self) -> bool {
        self.save_pending
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub(super) fn durability_warning(&self) -> bool {
        self.durability_warning
    }

    pub(super) fn draft_is_dirty(&self, note_id: &str) -> bool {
        self.drafts
            .get(note_id)
            .zip(self.document.note(note_id))
            .is_some_and(|(draft, note)| draft.differs_from(note))
    }

    pub(super) fn ensure_draft(&mut self, note_id: &str) {
        if self.drafts.contains_key(note_id) {
            return;
        }
        if let Some(note) = self.document.note(note_id) {
            self.drafts
                .insert(note_id.to_owned(), NoteDraft::from_note(note));
        }
    }

    pub(super) fn new_item_draft_mut(
        &mut self,
        note_id: &str,
    ) -> &mut BoundedText<NOTES_ITEM_MAX_BYTES> {
        self.new_item_drafts.entry(note_id.to_owned()).or_default()
    }

    #[cfg(test)]
    pub(crate) fn set_new_item_draft_for_tests(&mut self, note_id: &str, value: &str) {
        self.new_item_draft_mut(note_id).set(value);
    }

    #[cfg(test)]
    pub(crate) fn new_item_draft_for_tests(&self, note_id: &str) -> Option<&str> {
        self.new_item_drafts.get(note_id).map(BoundedText::as_str)
    }

    #[cfg(test)]
    fn active_draft(&self) -> Option<&NoteDraft> {
        self.drafts.get(&self.document.active_note_id)
    }

    fn dirty_note_ids(&self) -> Vec<String> {
        self.drafts
            .iter()
            .filter_map(|(id, draft)| {
                self.document
                    .note(id)
                    .filter(|note| draft.differs_from(note))
                    .map(|_| id.clone())
            })
            .collect()
    }

    fn reconcile_drafts(&mut self, preserve: &[String]) {
        self.drafts.retain(|id, _| self.document.note(id).is_some());
        self.new_item_drafts
            .retain(|id, _| self.document.note(id).is_some());
        let note_ids = self
            .document
            .notes
            .iter()
            .map(|note| note.id.clone())
            .collect::<Vec<_>>();
        for id in note_ids {
            if preserve.iter().any(|preserved| preserved == &id) {
                continue;
            }
            if let Some(note) = self.document.note(&id) {
                self.drafts.insert(id, NoteDraft::from_note(note));
            }
        }
    }

    fn restore_failed_inputs(&mut self, dirty: &[String]) {
        for (id, submitted) in &self.pending_note_drafts {
            if self.document.note(id).is_some() && !dirty.iter().any(|dirty_id| dirty_id == id) {
                self.drafts.insert(id.clone(), submitted.clone());
            }
        }
        for (note_id, submitted) in &self.pending_item_additions {
            if self.document.note(note_id).is_none() {
                continue;
            }
            let draft = self.new_item_drafts.entry(note_id.clone()).or_default();
            if draft.as_str().is_empty() {
                *draft = submitted.clone();
            }
        }
    }
}

fn bound_bytes(value: &str, max_bytes: usize) -> String {
    bounded_prefix(value, max_bytes).to_owned()
}

#[derive(Clone, Debug, Default)]
pub(super) struct BoundedText<const MAX_BYTES: usize>(String);

impl<const MAX_BYTES: usize> BoundedText<MAX_BYTES> {
    fn set(&mut self, value: &str) {
        self.0 = bound_bytes(value, MAX_BYTES);
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

impl<const MAX_BYTES: usize> TextBuffer for BoundedText<MAX_BYTES> {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        let remaining = MAX_BYTES.saturating_sub(self.0.len());
        let text = bounded_prefix(text, remaining);
        let inserted = text.chars().count();
        let byte_index = byte_index_from_char_index(&self.0, char_index.0);
        self.0.insert_str(byte_index, text);
        inserted
    }

    fn delete_char_range(&mut self, char_range: Range<CharIndex>) {
        if char_range.start > char_range.end {
            return;
        }
        let start = byte_index_from_char_index(&self.0, char_range.start.0);
        let end = byte_index_from_char_index(&self.0, char_range.end.0);
        self.0.drain(start..end);
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

fn bounded_prefix(value: &str, max_bytes: usize) -> &str {
    let mut boundary = value.len().min(max_bytes);
    while !value.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    &value[..boundary]
}

fn byte_index_from_char_index(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map_or(value.len(), |(byte_index, _)| byte_index)
}

fn bound_chars(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value;
    }
    let mut bounded = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    bounded.push('…');
    bounded
}
