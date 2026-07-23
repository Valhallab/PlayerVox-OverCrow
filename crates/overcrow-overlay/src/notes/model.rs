use std::{collections::HashSet, error::Error, fmt, io};

use serde::{Deserialize, Serialize};

pub const NOTES_SCHEMA_VERSION: u32 = 1;
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
pub struct NotesDocument {
    pub schema_version: u32,
    pub id: String,
    pub provider: NotesProviderRef,
    pub revision: u64,
    pub next_local_id: u64,
    pub note: String,
    pub items: Vec<ChecklistItem>,
}

impl Default for NotesDocument {
    fn default() -> Self {
        Self {
            schema_version: NOTES_SCHEMA_VERSION,
            id: "global".to_owned(),
            provider: NotesProviderRef::default(),
            revision: 0,
            next_local_id: 1,
            note: String::new(),
            items: Vec::new(),
        }
    }
}

impl NotesDocument {
    pub fn validate(&self) -> Result<(), NotesError> {
        if self.schema_version != NOTES_SCHEMA_VERSION {
            return Err(NotesError::validation("unsupported notes schema version"));
        }
        validate_identifier(&self.id, "document ID")?;
        if self.id != "global" {
            return Err(NotesError::validation("notes document ID must be global"));
        }
        validate_identifier(&self.provider.kind, "provider kind")?;
        if let Some(remote_id) = &self.provider.remote_id {
            validate_identifier(remote_id, "provider remote ID")?;
        }
        if self.provider.kind != "local" || self.provider.remote_id.is_some() {
            return Err(NotesError::validation(
                "local notes require kind local and no remote ID",
            ));
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
        if self.next_local_id == 0 {
            return Err(NotesError::validation("next local ID must be positive"));
        }

        let mut identifiers = HashSet::with_capacity(self.items.len());
        for item in &self.items {
            validate_identifier(&item.id, "checklist item ID")?;
            let number = local_identifier_number(&item.id)?;
            if number >= self.next_local_id {
                return Err(NotesError::validation(
                    "checklist item ID must precede the next local ID",
                ));
            }
            if !identifiers.insert(&item.id) {
                return Err(NotesError::validation("duplicate checklist item ID"));
            }
            if item.text.len() > NOTES_ITEM_MAX_BYTES {
                return Err(NotesError::validation(format!(
                    "checklist item exceeds {NOTES_ITEM_MAX_BYTES} UTF-8 bytes"
                )));
            }
        }
        Ok(())
    }

    pub fn set_note(&mut self, note: impl Into<String>) -> Result<(), NotesError> {
        self.mutate(|candidate| {
            candidate.note = note.into();
            Ok(())
        })
    }

    pub fn add_item(&mut self, text: impl Into<String>) -> Result<String, NotesError> {
        self.validate()?;
        let mut candidate = self.clone();
        let id = format!("local-{}", candidate.next_local_id);
        candidate.next_local_id = candidate
            .next_local_id
            .checked_add(1)
            .ok_or_else(|| NotesError::validation("local item ID counter overflow"))?;
        candidate.items.push(ChecklistItem {
            id: id.clone(),
            text: text.into(),
            checked: false,
        });
        candidate.increment_revision()?;
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }

    pub fn set_item_text(&mut self, id: &str, text: impl Into<String>) -> Result<(), NotesError> {
        validate_identifier(id, "checklist item ID")?;
        self.mutate(|candidate| {
            let item = candidate.item_mut(id)?;
            item.text = text.into();
            Ok(())
        })
    }

    pub fn set_checked(&mut self, id: &str, checked: bool) -> Result<(), NotesError> {
        validate_identifier(id, "checklist item ID")?;
        self.mutate(|candidate| {
            candidate.item_mut(id)?.checked = checked;
            Ok(())
        })
    }

    pub fn remove_item(&mut self, id: &str) -> Result<(), NotesError> {
        validate_identifier(id, "checklist item ID")?;
        self.mutate(|candidate| {
            let index = candidate
                .items
                .iter()
                .position(|item| item.id == id)
                .ok_or_else(|| NotesError::validation("unknown checklist item ID"))?;
            candidate.items.remove(index);
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

    fn item_mut(&mut self, id: &str) -> Result<&mut ChecklistItem, NotesError> {
        self.items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| NotesError::validation("unknown checklist item ID"))
    }
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

fn local_identifier_number(identifier: &str) -> Result<u64, NotesError> {
    let suffix = identifier
        .strip_prefix("local-")
        .ok_or_else(|| NotesError::validation("checklist item ID must use local-N form"))?;
    let number = suffix
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0 && number.to_string() == suffix)
        .ok_or_else(|| NotesError::validation("checklist item ID must use local-N form"))?;
    Ok(number)
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
