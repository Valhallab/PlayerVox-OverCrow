// Task 12 consumes this crate-private seam; Task 11 establishes it independently.
#![allow(dead_code)]

mod model;
mod service;
mod store;

#[allow(unused_imports)]
pub use model::{
    ChecklistItem, NOTES_IDENTIFIER_MAX_BYTES, NOTES_ITEM_MAX_BYTES, NOTES_ITEM_MAX_COUNT,
    NOTES_NOTE_MAX_BYTES, NOTES_SCHEMA_VERSION, NotesDocument, NotesError, NotesProviderRef,
};
#[allow(unused_imports)]
pub use service::{NotesCommand, NotesService, NotesUpdate};
#[allow(unused_imports)]
pub use store::{LocalNotesRepository, NOTES_FILE_MAX_BYTES, NotesRepository};

#[cfg(test)]
mod tests;
