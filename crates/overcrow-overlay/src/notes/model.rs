use std::{collections::HashSet, error::Error, fmt, io};

use serde::{Deserialize, Serialize};

pub const NOTES_SCHEMA_VERSION: u32 = 2;
pub const NOTES_PAGE_MAX_COUNT: usize = 8;
pub const NOTES_TITLE_MAX_BYTES: usize = 96;
pub const NOTES_NOTE_MAX_BYTES: usize = 8 * 1024;
pub const NOTES_ITEM_MAX_COUNT: usize = 64;
pub const NOTES_ITEM_MAX_BYTES: usize = 256;
pub const NOTES_IDENTIFIER_MAX_BYTES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotesProviderRef {
    pub kind: String,
    pub remote_id: Option<String>,
}

impl Default for NotesProviderRef {
    fn default() -> Self {
        Self {
            kind: "local".to_owned(),
            remote_id: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecklistItem {
    pub id: String,
    pub text: String,
    pub checked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotePage {
    pub id: String,
    pub title: String,
    pub body: String,
    pub items: Vec<ChecklistItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotesDocument {
    pub schema_version: u32,
    pub id: String,
    pub provider: NotesProviderRef,
    pub revision: u64,
    pub next_note_id: u64,
    pub next_item_id: u64,
    pub active_note_id: String,
    pub notes: Vec<NotePage>,
}

impl Default for NotesDocument {
    fn default() -> Self {
        Self {
            schema_version: NOTES_SCHEMA_VERSION,
            id: "global".to_owned(),
            provider: NotesProviderRef::default(),
            revision: 0,
            next_note_id: 2,
            next_item_id: 1,
            active_note_id: "note-1".to_owned(),
            notes: vec![NotePage {
                id: "note-1".to_owned(),
                title: "General".to_owned(),
                body: String::new(),
                items: Vec::new(),
            }],
        }
    }
}

impl NotesDocument {
    pub fn validate(&self) -> Result<(), NotesError> {
        if self.schema_version != NOTES_SCHEMA_VERSION {
            return Err(NotesError::validation("unsupported notes schema version"));
        }
        validate_local_identity(&self.id, &self.provider)?;
        if self.notes.is_empty() {
            return Err(NotesError::validation(
                "notes document must contain at least one note",
            ));
        }
        if self.notes.len() > NOTES_PAGE_MAX_COUNT {
            return Err(NotesError::validation(format!(
                "notes document exceeds {NOTES_PAGE_MAX_COUNT} notes"
            )));
        }
        if self.next_note_id == 0 || self.next_item_id == 0 {
            return Err(NotesError::validation(
                "next note and item IDs must be positive",
            ));
        }

        let mut note_ids = HashSet::with_capacity(self.notes.len());
        let mut item_ids = HashSet::new();
        for note in &self.notes {
            validate_identifier(&note.id, "note ID")?;
            let number = numbered_identifier(&note.id, "note-", "note ID")?;
            if number >= self.next_note_id {
                return Err(NotesError::validation(
                    "note ID must precede the next note ID",
                ));
            }
            if !note_ids.insert(&note.id) {
                return Err(NotesError::validation("duplicate note ID"));
            }
            validate_title(&note.title)?;
            if note.body.len() > NOTES_NOTE_MAX_BYTES {
                return Err(NotesError::validation(format!(
                    "note exceeds {NOTES_NOTE_MAX_BYTES} UTF-8 bytes"
                )));
            }
            if note.items.len() > NOTES_ITEM_MAX_COUNT {
                return Err(NotesError::validation(format!(
                    "checklist exceeds {NOTES_ITEM_MAX_COUNT} items"
                )));
            }
            for item in &note.items {
                validate_identifier(&item.id, "checklist item ID")?;
                let number = numbered_identifier(&item.id, "local-", "checklist item ID")?;
                if number >= self.next_item_id {
                    return Err(NotesError::validation(
                        "checklist item ID must precede the next item ID",
                    ));
                }
                if !item_ids.insert(&item.id) {
                    return Err(NotesError::validation("duplicate checklist item ID"));
                }
                if item.text.len() > NOTES_ITEM_MAX_BYTES {
                    return Err(NotesError::validation(format!(
                        "checklist item exceeds {NOTES_ITEM_MAX_BYTES} UTF-8 bytes"
                    )));
                }
            }
        }
        if !note_ids.contains(&self.active_note_id) {
            return Err(NotesError::validation("active note ID is unknown"));
        }
        Ok(())
    }

    pub fn active_note(&self) -> Option<&NotePage> {
        self.note(&self.active_note_id)
    }

    pub fn note(&self, note_id: &str) -> Option<&NotePage> {
        self.notes.iter().find(|note| note.id == note_id)
    }

    pub fn add_note(&mut self, title: impl Into<String>) -> Result<String, NotesError> {
        self.validate()?;
        let mut candidate = self.clone();
        let id = format!("note-{}", candidate.next_note_id);
        candidate.next_note_id = candidate
            .next_note_id
            .checked_add(1)
            .ok_or_else(|| NotesError::validation("local note ID counter overflow"))?;
        candidate.notes.push(NotePage {
            id: id.clone(),
            title: title.into(),
            body: String::new(),
            items: Vec::new(),
        });
        candidate.active_note_id.clone_from(&id);
        candidate.increment_revision()?;
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }

    pub fn set_active_note(&mut self, note_id: &str) -> Result<(), NotesError> {
        validate_identifier(note_id, "note ID")?;
        self.validate()?;
        if self.active_note_id == note_id {
            return Ok(());
        }
        self.mutate(|candidate| {
            if candidate.note(note_id).is_none() {
                return Err(NotesError::validation("unknown note ID"));
            }
            candidate.active_note_id = note_id.to_owned();
            Ok(())
        })
    }

    pub fn update_note(
        &mut self,
        note_id: &str,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<(), NotesError> {
        validate_identifier(note_id, "note ID")?;
        self.mutate(|candidate| {
            let note = candidate.note_mut(note_id)?;
            note.title = title.into();
            note.body = body.into();
            Ok(())
        })
    }

    #[cfg(test)]
    pub fn set_note(&mut self, body: impl Into<String>) -> Result<(), NotesError> {
        let note_id = self.active_note_id.clone();
        let title = self
            .active_note()
            .ok_or_else(|| NotesError::validation("active note ID is unknown"))?
            .title
            .clone();
        self.update_note(&note_id, title, body)
    }

    pub fn remove_note(&mut self, note_id: &str) -> Result<(), NotesError> {
        validate_identifier(note_id, "note ID")?;
        self.mutate(|candidate| {
            let index = candidate
                .notes
                .iter()
                .position(|note| note.id == note_id)
                .ok_or_else(|| NotesError::validation("unknown note ID"))?;
            if candidate.notes.len() == 1 {
                return Err(NotesError::validation("cannot remove the last note"));
            }
            candidate.notes.remove(index);
            if candidate.active_note_id == note_id {
                let replacement = index.min(candidate.notes.len().saturating_sub(1));
                candidate.active_note_id = candidate.notes[replacement].id.clone();
            }
            Ok(())
        })
    }

    #[cfg(test)]
    pub fn add_item(&mut self, text: impl Into<String>) -> Result<String, NotesError> {
        let note_id = self.active_note_id.clone();
        self.add_item_to(&note_id, text)
    }

    pub fn add_item_to(
        &mut self,
        note_id: &str,
        text: impl Into<String>,
    ) -> Result<String, NotesError> {
        validate_identifier(note_id, "note ID")?;
        self.validate()?;
        let mut candidate = self.clone();
        let id = format!("local-{}", candidate.next_item_id);
        candidate.next_item_id = candidate
            .next_item_id
            .checked_add(1)
            .ok_or_else(|| NotesError::validation("local item ID counter overflow"))?;
        candidate.note_mut(note_id)?.items.push(ChecklistItem {
            id: id.clone(),
            text: text.into(),
            checked: false,
        });
        candidate.increment_revision()?;
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }

    #[cfg(test)]
    pub fn set_item_text(
        &mut self,
        item_id: &str,
        text: impl Into<String>,
    ) -> Result<(), NotesError> {
        let note_id = self.active_note_id.clone();
        self.set_item_text_in(&note_id, item_id, text)
    }

    pub fn set_item_text_in(
        &mut self,
        note_id: &str,
        item_id: &str,
        text: impl Into<String>,
    ) -> Result<(), NotesError> {
        validate_identifier(note_id, "note ID")?;
        validate_identifier(item_id, "checklist item ID")?;
        self.mutate(|candidate| {
            candidate.item_mut(note_id, item_id)?.text = text.into();
            Ok(())
        })
    }

    #[cfg(test)]
    pub fn set_checked(&mut self, item_id: &str, checked: bool) -> Result<(), NotesError> {
        let note_id = self.active_note_id.clone();
        self.set_checked_in(&note_id, item_id, checked)
    }

    pub fn set_checked_in(
        &mut self,
        note_id: &str,
        item_id: &str,
        checked: bool,
    ) -> Result<(), NotesError> {
        validate_identifier(note_id, "note ID")?;
        validate_identifier(item_id, "checklist item ID")?;
        self.mutate(|candidate| {
            candidate.item_mut(note_id, item_id)?.checked = checked;
            Ok(())
        })
    }

    #[cfg(test)]
    pub fn remove_item(&mut self, item_id: &str) -> Result<(), NotesError> {
        let note_id = self.active_note_id.clone();
        self.remove_item_from(&note_id, item_id)
    }

    pub fn remove_item_from(&mut self, note_id: &str, item_id: &str) -> Result<(), NotesError> {
        validate_identifier(note_id, "note ID")?;
        validate_identifier(item_id, "checklist item ID")?;
        self.mutate(|candidate| {
            let items = &mut candidate.note_mut(note_id)?.items;
            let index = items
                .iter()
                .position(|item| item.id == item_id)
                .ok_or_else(|| NotesError::validation("unknown checklist item ID"))?;
            items.remove(index);
            Ok(())
        })
    }

    fn mutate(
        &mut self,
        mutation: impl FnOnce(&mut NotesDocument) -> Result<(), NotesError>,
    ) -> Result<(), NotesError> {
        self.validate()?;
        let mut candidate = self.clone();
        mutation(&mut candidate)?;
        candidate.increment_revision()?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    fn increment_revision(&mut self) -> Result<(), NotesError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| NotesError::validation("notes revision overflow"))?;
        Ok(())
    }

    fn note_mut(&mut self, note_id: &str) -> Result<&mut NotePage, NotesError> {
        self.notes
            .iter_mut()
            .find(|note| note.id == note_id)
            .ok_or_else(|| NotesError::validation("unknown note ID"))
    }

    fn item_mut(&mut self, note_id: &str, item_id: &str) -> Result<&mut ChecklistItem, NotesError> {
        self.note_mut(note_id)?
            .items
            .iter_mut()
            .find(|item| item.id == item_id)
            .ok_or_else(|| NotesError::validation("unknown checklist item ID"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyNotesDocument {
    schema_version: u32,
    id: String,
    provider: NotesProviderRef,
    revision: u64,
    next_local_id: u64,
    note: String,
    items: Vec<ChecklistItem>,
}

impl LegacyNotesDocument {
    pub(super) fn migrate(self) -> Result<NotesDocument, NotesError> {
        if self.schema_version != 1 {
            return Err(NotesError::validation("unsupported notes schema version"));
        }
        validate_local_identity(&self.id, &self.provider)?;
        if self.next_local_id == 0 {
            return Err(NotesError::validation("next local ID must be positive"));
        }
        if self.note.len() > NOTES_NOTE_MAX_BYTES {
            return Err(NotesError::validation(format!(
                "note exceeds {NOTES_NOTE_MAX_BYTES} UTF-8 bytes"
            )));
        }
        if self.items.len() > NOTES_ITEM_MAX_COUNT {
            return Err(NotesError::validation(format!(
                "checklist exceeds {NOTES_ITEM_MAX_COUNT} items"
            )));
        }
        let mut item_ids = HashSet::with_capacity(self.items.len());
        for item in &self.items {
            validate_identifier(&item.id, "checklist item ID")?;
            let number = numbered_identifier(&item.id, "local-", "checklist item ID")?;
            if number >= self.next_local_id {
                return Err(NotesError::validation(
                    "checklist item ID must precede the next local ID",
                ));
            }
            if !item_ids.insert(&item.id) {
                return Err(NotesError::validation("duplicate checklist item ID"));
            }
            if item.text.len() > NOTES_ITEM_MAX_BYTES {
                return Err(NotesError::validation(format!(
                    "checklist item exceeds {NOTES_ITEM_MAX_BYTES} UTF-8 bytes"
                )));
            }
        }

        let document = NotesDocument {
            schema_version: NOTES_SCHEMA_VERSION,
            id: self.id,
            provider: self.provider,
            revision: self.revision,
            next_note_id: 2,
            next_item_id: self.next_local_id,
            active_note_id: "note-1".to_owned(),
            notes: vec![NotePage {
                id: "note-1".to_owned(),
                title: "General".to_owned(),
                body: self.note,
                items: self.items,
            }],
        };
        document.validate()?;
        Ok(document)
    }
}

fn validate_local_identity(id: &str, provider: &NotesProviderRef) -> Result<(), NotesError> {
    validate_identifier(id, "document ID")?;
    if id != "global" {
        return Err(NotesError::validation("notes document ID must be global"));
    }
    validate_identifier(&provider.kind, "provider kind")?;
    if let Some(remote_id) = &provider.remote_id {
        validate_identifier(remote_id, "provider remote ID")?;
    }
    if provider.kind != "local" || provider.remote_id.is_some() {
        return Err(NotesError::validation(
            "local notes require kind local and no remote ID",
        ));
    }
    Ok(())
}

fn validate_title(title: &str) -> Result<(), NotesError> {
    if title.trim().is_empty() {
        return Err(NotesError::validation("note title must not be empty"));
    }
    if title.len() > NOTES_TITLE_MAX_BYTES {
        return Err(NotesError::validation(format!(
            "note title exceeds {NOTES_TITLE_MAX_BYTES} UTF-8 bytes"
        )));
    }
    Ok(())
}

fn validate_identifier(identifier: &str, description: &str) -> Result<(), NotesError> {
    if identifier.is_empty() {
        return Err(NotesError::validation(format!(
            "{description} must not be empty"
        )));
    }
    if identifier.len() > NOTES_IDENTIFIER_MAX_BYTES {
        return Err(NotesError::validation(format!(
            "{description} exceeds {NOTES_IDENTIFIER_MAX_BYTES} UTF-8 bytes"
        )));
    }
    Ok(())
}

fn numbered_identifier(
    identifier: &str,
    prefix: &str,
    description: &str,
) -> Result<u64, NotesError> {
    let suffix = identifier
        .strip_prefix(prefix)
        .ok_or_else(|| NotesError::validation(format!("{description} has an invalid form")))?;
    suffix
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0 && number.to_string() == suffix)
        .ok_or_else(|| NotesError::validation(format!("{description} has an invalid form")))
}

#[derive(Debug)]
pub enum NotesError {
    Validation(String),
    Repository(String),
    Io(io::Error),
    Json(serde_json::Error),
    Committed(io::Error),
}

impl NotesError {
    pub fn repository(message: impl Into<String>) -> Self {
        Self::Repository(message.into())
    }

    pub fn committed(source: io::Error) -> Self {
        Self::Committed(source)
    }

    pub fn was_committed(&self) -> bool {
        matches!(self, Self::Committed(_))
    }

    pub(crate) fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }
}

impl fmt::Display for NotesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(message) => write!(formatter, "invalid notes document: {message}"),
            Self::Repository(message) => write!(formatter, "notes repository failed: {message}"),
            Self::Io(source) => write!(formatter, "notes I/O failed: {source}"),
            Self::Json(source) => write!(formatter, "invalid notes JSON: {source}"),
            Self::Committed(source) => write!(
                formatter,
                "notes were replaced but parent directory sync failed: {source}"
            ),
        }
    }
}

impl Error for NotesError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(source) | Self::Committed(source) => Some(source),
            Self::Json(source) => Some(source),
            Self::Validation(_) | Self::Repository(_) => None,
        }
    }
}

impl From<io::Error> for NotesError {
    fn from(source: io::Error) -> Self {
        Self::Io(source)
    }
}

impl From<serde_json::Error> for NotesError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json(source)
    }
}
