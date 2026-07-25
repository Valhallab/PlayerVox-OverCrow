mod model;
mod service;
mod store;

#[allow(unused_imports)]
pub use model::{
    ChecklistItem, NOTES_IDENTIFIER_MAX_BYTES, NOTES_ITEM_MAX_BYTES, NOTES_ITEM_MAX_COUNT,
    NOTES_NOTE_MAX_BYTES, NOTES_PAGE_MAX_COUNT, NOTES_SCHEMA_VERSION, NOTES_TITLE_MAX_BYTES,
    NotePage, NotesDocument, NotesError, NotesProviderRef,
};
#[allow(unused_imports)]
pub use service::{NotesCommand, NotesService, NotesUpdate};
#[allow(unused_imports)]
pub use store::{LocalNotesRepository, NOTES_FILE_MAX_BYTES, NotesRepository};

#[cfg(test)]
mod tests;
