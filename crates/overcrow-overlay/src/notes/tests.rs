use std::{
    ffi::{CString, OsStr},
    fs, io,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, Instant},
};

use serde_json::json;
use tempfile::NamedTempFile;

use super::{
    ChecklistItem, LocalNotesRepository, NOTES_FILE_MAX_BYTES, NOTES_IDENTIFIER_MAX_BYTES,
    NOTES_ITEM_MAX_BYTES, NOTES_ITEM_MAX_COUNT, NOTES_NOTE_MAX_BYTES, NOTES_SCHEMA_VERSION,
    NotesCommand, NotesDocument, NotesError, NotesProviderRef, NotesRepository, NotesService,
    NotesUpdate,
    store::{AtomicWriter, notes_path},
};

#[test]
fn default_document_has_the_exact_local_identity_and_source() {
    let document = NotesDocument::default();

    assert_eq!(document.schema_version, NOTES_SCHEMA_VERSION);
    assert_eq!(document.id, "global");
    assert_eq!(
        document.provider,
        NotesProviderRef {
            kind: "local".to_owned(),
            remote_id: None,
        }
    );
    assert_eq!(document.revision, 0);
    assert_eq!(document.next_local_id, 1);
    assert!(document.note.is_empty());
    assert!(document.items.is_empty());
    assert!(document.validate().is_ok());
}

#[test]
fn checked_mutation_increments_revision_without_changing_item_identity() {
    let mut document = NotesDocument::default();
    let id = document.add_item("Find the shrine").unwrap();

    document.set_checked(&id, true).unwrap();

    assert_eq!(document.revision, 2);
    assert_eq!(document.items[0].id, id);
    assert!(document.items[0].checked);
}

#[test]
fn local_ids_are_monotonic_and_never_reused_after_deletion() {
    let mut document = NotesDocument::default();
    let first = document.add_item("first").unwrap();
    let second = document.add_item("second").unwrap();
    document.remove_item(&second).unwrap();
    let third = document.add_item("third").unwrap();

    assert_eq!(first, "local-1");
    assert_eq!(second, "local-2");
    assert_eq!(third, "local-3");
    assert_eq!(document.next_local_id, 4);
}

#[test]
fn every_successful_mutation_increments_revision_once() {
    let mut document = NotesDocument::default();
    document.set_note("plain <b>text</b>").unwrap();
    let id = document.add_item("original").unwrap();
    document.set_item_text(&id, "edited").unwrap();
    document.set_checked(&id, true).unwrap();
    document.remove_item(&id).unwrap();

    assert_eq!(document.revision, 5);
    assert_eq!(document.note, "plain <b>text</b>");
}

#[test]
fn note_limit_is_exact_utf8_bytes_and_failed_mutation_is_atomic() {
    let mut document = NotesDocument::default();
    document.set_note("a".repeat(NOTES_NOTE_MAX_BYTES)).unwrap();
    let committed = document.clone();

    let error = document
        .set_note(format!("{}é", "a".repeat(NOTES_NOTE_MAX_BYTES - 1)))
        .unwrap_err();

    assert!(matches!(error, NotesError::Validation(_)));
    assert_eq!(document, committed);
}

#[test]
fn item_text_limit_is_exact_utf8_bytes_and_failed_mutation_is_atomic() {
    let mut document = NotesDocument::default();
    let id = document.add_item("a".repeat(NOTES_ITEM_MAX_BYTES)).unwrap();
    let committed = document.clone();

    let error = document
        .set_item_text(&id, format!("{}é", "a".repeat(NOTES_ITEM_MAX_BYTES - 1)))
        .unwrap_err();

    assert!(matches!(error, NotesError::Validation(_)));
    assert_eq!(document, committed);
}

#[test]
fn checklist_count_limit_is_exact_and_failed_add_is_atomic() {
    let mut document = NotesDocument::default();
    for index in 0..NOTES_ITEM_MAX_COUNT {
        assert_eq!(
            document.add_item(index.to_string()).unwrap(),
            format!("local-{}", index + 1)
        );
    }
    let committed = document.clone();

    assert!(matches!(
        document.add_item("one too many"),
        Err(NotesError::Validation(_))
    ));
    assert_eq!(document, committed);
}

#[test]
fn identifier_limit_and_exact_local_source_are_validated() {
    let oversized_document_id = NotesDocument {
        id: "x".repeat(NOTES_IDENTIFIER_MAX_BYTES + 1),
        ..NotesDocument::default()
    };
    assert!(matches!(
        oversized_document_id.validate(),
        Err(NotesError::Validation(_))
    ));

    let mut oversized_provider = NotesDocument::default();
    oversized_provider.provider.kind = "x".repeat(NOTES_IDENTIFIER_MAX_BYTES + 1);
    assert!(matches!(
        oversized_provider.validate(),
        Err(NotesError::Validation(_))
    ));

    let mut oversized_item = NotesDocument::default();
    oversized_item.items.push(ChecklistItem {
        id: "x".repeat(NOTES_IDENTIFIER_MAX_BYTES + 1),
        text: String::new(),
        checked: false,
    });
    assert!(matches!(
        oversized_item.validate(),
        Err(NotesError::Validation(_))
    ));

    for invalid_provider in [
        NotesProviderRef {
            kind: "api".to_owned(),
            remote_id: None,
        },
        NotesProviderRef {
            kind: "local".to_owned(),
            remote_id: Some("remote".to_owned()),
        },
    ] {
        let document = NotesDocument {
            provider: invalid_provider,
            ..NotesDocument::default()
        };
        assert!(matches!(
            document.validate(),
            Err(NotesError::Validation(_))
        ));
    }
}

#[test]
fn validation_rejects_schema_identity_duplicate_ids_and_invalid_counters() {
    let invalid_documents = [
        NotesDocument {
            schema_version: NOTES_SCHEMA_VERSION + 1,
            ..NotesDocument::default()
        },
        NotesDocument {
            id: "another".to_owned(),
            ..NotesDocument::default()
        },
        NotesDocument {
            next_local_id: 0,
            ..NotesDocument::default()
        },
        NotesDocument {
            next_local_id: 2,
            items: vec![
                ChecklistItem {
                    id: "local-1".to_owned(),
                    text: "one".to_owned(),
                    checked: false,
                },
                ChecklistItem {
                    id: "local-1".to_owned(),
                    text: "duplicate".to_owned(),
                    checked: false,
                },
            ],
            ..NotesDocument::default()
        },
        NotesDocument {
            next_local_id: 2,
            items: vec![ChecklistItem {
                id: "local-2".to_owned(),
                text: String::new(),
                checked: false,
            }],
            ..NotesDocument::default()
        },
    ];

    for document in invalid_documents {
        assert!(matches!(
            document.validate(),
            Err(NotesError::Validation(_))
        ));
    }
}

#[test]
fn unknown_fields_are_rejected_at_every_schema_level() {
    let document = serde_json::to_value(NotesDocument::default()).unwrap();
    let mut top = document.clone();
    top["unexpected"] = json!(true);
    let mut provider = document.clone();
    provider["provider"]["unexpected"] = json!(true);
    let mut item = document;
    item["items"] = json!([{"id":"local-1","text":"x","checked":false,"unexpected":true}]);
    item["next_local_id"] = json!(2);

    for invalid in [top, provider, item] {
        assert!(serde_json::from_value::<NotesDocument>(invalid).is_err());
    }
}

#[test]
fn plain_utf8_content_round_trips_without_interpretation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested/global.json");
    let repository = LocalNotesRepository::from_path(&path);
    let mut document = NotesDocument::default();
    document
        .set_note("# titre\n<script>alert('non')</script>\n🎮")
        .unwrap();
    document
        .add_item("[lien](https://example.invalid) & café")
        .unwrap();

    repository.save(&document).unwrap();

    assert_eq!(repository.load().unwrap(), document);
    assert!(fs::read_to_string(path).unwrap().contains("<script>"));
}

#[test]
fn notes_paths_prefer_absolute_xdg_then_absolute_home() {
    assert_eq!(
        notes_path(Some(OsStr::new("/xdg")), Some(OsStr::new("/home/player"))),
        PathBuf::from("/xdg/overcrow/notes/global.json")
    );
    assert_eq!(
        notes_path(None, Some(OsStr::new("/home/player"))),
        PathBuf::from("/home/player/.local/share/overcrow/notes/global.json")
    );
    assert_eq!(
        notes_path(
            Some(OsStr::new("relative")),
            Some(OsStr::new("/home/player"))
        ),
        PathBuf::from("/home/player/.local/share/overcrow/notes/global.json")
    );
    assert_eq!(notes_path(None, None), PathBuf::new());
}

#[test]
fn missing_file_loads_the_default_and_save_writes_private_regular_json() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested/global.json");
    let repository = LocalNotesRepository::from_path(&path);
    assert_eq!(repository.load().unwrap(), NotesDocument::default());

    let mut document = NotesDocument::default();
    document.set_note("saved").unwrap();
    repository.save(&document).unwrap();

    let metadata = fs::metadata(&path).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
    assert!(fs::read(&path).unwrap().ends_with(b"\n"));
}

#[test]
fn load_rejects_oversized_files_before_json_parsing() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("global.json");
    write_private(&path, &vec![b' '; NOTES_FILE_MAX_BYTES + 1]);

    let error = LocalNotesRepository::from_path(path).load().unwrap_err();

    assert!(error.to_string().contains("too large"));
}

#[test]
fn load_accepts_a_valid_document_at_the_exact_file_size_limit() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("global.json");
    let mut contents = serde_json::to_vec(&NotesDocument::default()).unwrap();
    contents.resize(NOTES_FILE_MAX_BYTES, b' ');
    write_private(&path, &contents);

    assert_eq!(
        LocalNotesRepository::from_path(path).load().unwrap(),
        NotesDocument::default()
    );
}

#[test]
fn load_rejects_invalid_schema_content_without_replacing_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("global.json");
    let invalid = br#"{"schema_version":1,"id":"global","provider":{"kind":"api","remote_id":null},"revision":0,"next_local_id":1,"note":"","items":[]}"#;
    write_private(&path, invalid);

    let error = LocalNotesRepository::from_path(&path).load().unwrap_err();

    assert!(matches!(error, NotesError::Validation(_)));
    assert_eq!(fs::read(path).unwrap(), invalid);
}

#[test]
fn load_rejects_symlinks_fifos_and_non_private_modes_without_blocking() {
    for unsafe_kind in [
        UnsafeFile::Symlink,
        UnsafeFile::Fifo,
        UnsafeFile::PublicMode,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("global.json");
        make_unsafe_file(&path, unsafe_kind);
        let started = Instant::now();

        let error = LocalNotesRepository::from_path(path).load().unwrap_err();

        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(
            error.to_string().contains("unsafe"),
            "{unsafe_kind:?}: {error}"
        );
    }
}

#[test]
fn pre_replacement_save_failure_preserves_the_previous_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("global.json");
    let repository = LocalNotesRepository::from_path(&path);
    repository.save(&NotesDocument::default()).unwrap();
    let original = fs::read(&path).unwrap();
    let mut replacement = NotesDocument::default();
    replacement.set_note("replacement").unwrap();

    let error = repository
        .save_with_writer(&replacement, &FailingAtomicWriter::before_replace())
        .unwrap_err();

    assert!(!error.was_committed());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn parent_sync_failure_is_distinguished_after_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("global.json");
    let repository = LocalNotesRepository::from_path(&path);
    repository.save(&NotesDocument::default()).unwrap();
    let mut replacement = NotesDocument::default();
    replacement.set_note("replacement").unwrap();

    let error = repository
        .save_with_writer(&replacement, &FailingAtomicWriter::after_replace())
        .unwrap_err();

    assert!(error.was_committed());
    assert_eq!(repository.load().unwrap(), replacement);
}

#[test]
fn repository_rejects_invalid_documents_before_creating_directories() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("absent/global.json");
    let invalid = NotesDocument {
        note: "x".repeat(NOTES_NOTE_MAX_BYTES + 1),
        ..NotesDocument::default()
    };

    assert!(matches!(
        LocalNotesRepository::from_path(&path).save(&invalid),
        Err(NotesError::Validation(_))
    ));
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn service_applies_every_command_against_a_validated_document() {
    let repository = MemoryRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository.clone(), || {});
    wait_for_ready(&service);

    service
        .send(NotesCommand::SetNote("note".to_owned()))
        .unwrap();
    let saved = wait_for_settled(&service);
    assert_eq!(saved.document.note, "note");
    let id = saved.document.items.first().map(|item| item.id.clone());
    assert!(id.is_none());

    service
        .send(NotesCommand::AddItem("item".to_owned()))
        .unwrap();
    let saved = wait_for_settled(&service);
    let id = saved.document.items[0].id.clone();
    service
        .send(NotesCommand::SetItemText {
            id: id.clone(),
            text: "edited".to_owned(),
        })
        .unwrap();
    wait_for_settled(&service);
    service
        .send(NotesCommand::SetChecked {
            id: id.clone(),
            checked: true,
        })
        .unwrap();
    wait_for_settled(&service);
    service.send(NotesCommand::RemoveItem { id }).unwrap();
    let saved = wait_for_settled(&service);

    assert!(saved.document.items.is_empty());
    assert_eq!(saved.document.revision, 5);
    assert_eq!(repository.current(), saved.document);
}

#[test]
fn rejected_command_does_not_queue_or_change_state() {
    let repository = MemoryRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository.clone(), || {});
    let ready = wait_for_ready(&service);

    let error = service
        .send(NotesCommand::SetNote("x".repeat(NOTES_NOTE_MAX_BYTES + 1)))
        .unwrap_err();

    assert!(matches!(error, NotesError::Validation(_)));
    assert_eq!(service.current(), ready.document);
    assert_eq!(repository.save_count(), 0);
}

#[test]
fn disconnected_worker_rejects_without_publishing_a_candidate() {
    let service = NotesService::unavailable_for_tests();
    let committed = service.current();

    let error = service
        .send(NotesCommand::SetNote("must not leak".to_owned()))
        .unwrap_err();
    let update = service.take_latest().unwrap();

    assert!(error.to_string().contains("worker unavailable"));
    assert_eq!(service.current(), committed);
    assert_eq!(update.document, committed);
    assert!(!update.save_pending);
    assert!(update.error.unwrap().contains("worker unavailable"));
}

#[test]
fn pre_commit_save_failure_retains_the_last_committed_document() {
    let repository = MemoryRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository.clone(), || {});
    let committed = wait_for_ready(&service).document;
    repository.fail_next(false);

    service
        .send(NotesCommand::SetNote("must roll back".to_owned()))
        .unwrap();
    let update = wait_for_error(&service);

    assert_eq!(update.document, committed);
    assert!(!update.save_pending);
    assert!(!update.durability_warning);
    assert_eq!(service.current(), committed);
    assert_eq!(repository.current(), committed);
}

#[test]
fn post_replacement_warning_publishes_the_disk_document() {
    let repository = MemoryRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository.clone(), || {});
    wait_for_ready(&service);
    repository.fail_next(true);

    service
        .send(NotesCommand::SetNote("committed on disk".to_owned()))
        .unwrap();
    let update = wait_for_error(&service);

    assert_eq!(update.document.note, "committed on disk");
    assert!(!update.save_pending);
    assert!(update.durability_warning);
    assert_eq!(service.current(), update.document);
    assert_eq!(repository.current(), update.document);
}

#[test]
fn latest_pending_save_coalesces_intermediate_documents() {
    let repository = BlockingRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository.clone(), || {});
    wait_for_ready(&service);

    service
        .send(NotesCommand::SetNote("first".to_owned()))
        .unwrap();
    repository.wait_for_first_save();
    service
        .send(NotesCommand::SetNote("intermediate".to_owned()))
        .unwrap();
    service
        .send(NotesCommand::SetNote("latest".to_owned()))
        .unwrap();
    repository.release_first_save();
    let settled = wait_for_settled(&service);

    assert_eq!(settled.document.note, "latest");
    assert_eq!(
        repository.saved_notes(),
        ["first".to_owned(), "latest".to_owned()]
    );
}

#[test]
fn pending_candidate_is_not_published_before_repository_success() {
    let repository = BlockingRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository.clone(), || {});
    let committed = wait_for_ready(&service).document;

    service
        .send(NotesCommand::SetNote("not committed yet".to_owned()))
        .unwrap();
    repository.wait_for_first_save();
    let pending = wait_for_update(&service, |update| update.save_pending);
    let current_while_pending = service.current();
    repository.release_first_save();
    let settled = wait_for_settled(&service);

    assert_eq!(pending.document, committed);
    assert_eq!(current_while_pending, committed);
    assert_eq!(settled.document.note, "not committed yet");
}

#[test]
fn latest_result_publication_coalesces_pending_updates() {
    let repository = MemoryRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository, || {});
    wait_for_ready(&service);

    service
        .send(NotesCommand::SetNote("one".to_owned()))
        .unwrap();
    service
        .send(NotesCommand::SetNote("two".to_owned()))
        .unwrap();
    let update = wait_for_settled(&service);

    assert_eq!(update.document.note, "two");
    assert!(service.take_latest().is_none());
}

#[test]
fn service_load_failure_is_bounded_and_keeps_a_valid_default() {
    let repository = MemoryRepository::new(NotesDocument::default());
    repository.fail_load("x".repeat(1_000));
    let service = NotesService::spawn(repository, || {});
    let update = wait_for_error(&service);

    assert_eq!(update.document, NotesDocument::default());
    assert!(!update.save_pending);
    assert!(update.error.unwrap().chars().count() <= 180);
}

#[test]
fn dropping_service_wakes_and_joins_its_owned_named_worker() {
    let repository = MemoryRepository::new(NotesDocument::default());
    let dropped = repository.dropped_receiver();
    let observer = repository.clone();
    let service = NotesService::spawn(repository, || {});
    wait_for_ready(&service);
    assert_eq!(
        observer.load_thread_name().as_deref(),
        Some("overcrow-notes-provider")
    );
    drop(observer);
    let started = Instant::now();

    drop(service);

    assert!(started.elapsed() < Duration::from_millis(500));
    dropped.recv_timeout(Duration::from_millis(50)).unwrap();
}

fn wait_for_ready(service: &NotesService) -> NotesUpdate {
    wait_for_update(service, |update| {
        !update.save_pending && update.error.is_none()
    })
}

fn wait_for_settled(service: &NotesService) -> NotesUpdate {
    wait_for_update(service, |update| !update.save_pending)
}

fn wait_for_error(service: &NotesService) -> NotesUpdate {
    wait_for_update(service, |update| {
        !update.save_pending && update.error.is_some()
    })
}

fn wait_for_update(
    service: &NotesService,
    predicate: impl Fn(&NotesUpdate) -> bool,
) -> NotesUpdate {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(update) = service.take_latest()
            && predicate(&update)
        {
            return update;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for notes update"
        );
        std::thread::yield_now();
    }
}

fn write_private(path: &Path, contents: &[u8]) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[derive(Clone, Copy, Debug)]
enum UnsafeFile {
    Symlink,
    Fifo,
    PublicMode,
}

fn make_unsafe_file(path: &Path, kind: UnsafeFile) {
    match kind {
        UnsafeFile::Symlink => {
            let target = path.with_extension("target");
            write_private(&target, b"{}");
            std::os::unix::fs::symlink(target, path).unwrap();
        }
        UnsafeFile::Fifo => {
            let path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
            assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());
        }
        UnsafeFile::PublicMode => {
            fs::write(path, b"{}").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
}

struct FailingAtomicWriter {
    fail_before_replace: bool,
    fail_parent_sync: bool,
}

impl FailingAtomicWriter {
    fn before_replace() -> Self {
        Self {
            fail_before_replace: true,
            fail_parent_sync: false,
        }
    }

    fn after_replace() -> Self {
        Self {
            fail_before_replace: false,
            fail_parent_sync: true,
        }
    }
}

impl AtomicWriter for FailingAtomicWriter {
    fn write(&self, temporary: &mut NamedTempFile, contents: &[u8]) -> io::Result<()> {
        use io::Write;
        temporary.write_all(contents)
    }

    fn persist(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()> {
        if self.fail_before_replace {
            return Err(io::Error::other("forced pre-replacement failure"));
        }
        temporary
            .persist(destination)
            .map(|_| ())
            .map_err(|error| error.error)
    }

    fn sync_parent(&self, parent: &Path) -> io::Result<()> {
        if self.fail_parent_sync {
            return Err(io::Error::other("forced parent sync failure"));
        }
        fs::File::open(parent)?.sync_all()
    }
}

#[derive(Clone)]
struct MemoryRepository {
    shared: Arc<MemoryShared>,
}

struct MemoryShared {
    state: Mutex<MemoryState>,
    dropped_sender: mpsc::SyncSender<()>,
    dropped_receiver: Mutex<Option<mpsc::Receiver<()>>>,
}

struct MemoryState {
    current: NotesDocument,
    save_count: usize,
    next_failure: Option<bool>,
    load_failure: Option<String>,
    load_thread_name: Option<String>,
}

impl MemoryRepository {
    fn new(current: NotesDocument) -> Self {
        let (dropped_sender, dropped_receiver) = mpsc::sync_channel(1);
        Self {
            shared: Arc::new(MemoryShared {
                state: Mutex::new(MemoryState {
                    current,
                    save_count: 0,
                    next_failure: None,
                    load_failure: None,
                    load_thread_name: None,
                }),
                dropped_sender,
                dropped_receiver: Mutex::new(Some(dropped_receiver)),
            }),
        }
    }

    fn current(&self) -> NotesDocument {
        self.shared.state.lock().unwrap().current.clone()
    }

    fn save_count(&self) -> usize {
        self.shared.state.lock().unwrap().save_count
    }

    fn fail_next(&self, committed: bool) {
        self.shared.state.lock().unwrap().next_failure = Some(committed);
    }

    fn fail_load(&self, message: String) {
        self.shared.state.lock().unwrap().load_failure = Some(message);
    }

    fn load_thread_name(&self) -> Option<String> {
        self.shared.state.lock().unwrap().load_thread_name.clone()
    }

    fn dropped_receiver(&self) -> mpsc::Receiver<()> {
        self.shared.dropped_receiver.lock().unwrap().take().unwrap()
    }
}

impl NotesRepository for MemoryRepository {
    fn load(&self) -> Result<NotesDocument, NotesError> {
        let mut state = self.shared.state.lock().unwrap();
        state.load_thread_name = std::thread::current().name().map(str::to_owned);
        if let Some(message) = &state.load_failure {
            return Err(NotesError::repository(message));
        }
        Ok(state.current.clone())
    }

    fn save(&self, document: &NotesDocument) -> Result<(), NotesError> {
        let mut state = self.shared.state.lock().unwrap();
        state.save_count += 1;
        match state.next_failure.take() {
            Some(true) => {
                state.current = document.clone();
                Err(NotesError::committed(io::Error::other(
                    "forced parent sync failure",
                )))
            }
            Some(false) => Err(NotesError::repository("forced save failure")),
            None => {
                state.current = document.clone();
                Ok(())
            }
        }
    }
}

impl Drop for MemoryShared {
    fn drop(&mut self) {
        let _ = self.dropped_sender.send(());
    }
}

#[derive(Clone)]
struct BlockingRepository {
    shared: Arc<BlockingShared>,
}

struct BlockingShared {
    state: Mutex<BlockingState>,
    changed: Condvar,
}

struct BlockingState {
    current: NotesDocument,
    saved_notes: Vec<String>,
    first_started: bool,
    release_first: bool,
}

impl BlockingRepository {
    fn new(current: NotesDocument) -> Self {
        Self {
            shared: Arc::new(BlockingShared {
                state: Mutex::new(BlockingState {
                    current,
                    saved_notes: Vec::new(),
                    first_started: false,
                    release_first: false,
                }),
                changed: Condvar::new(),
            }),
        }
    }

    fn wait_for_first_save(&self) {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut state = self.shared.state.lock().unwrap();
        while !state.first_started {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let (next, wait) = self.shared.changed.wait_timeout(state, remaining).unwrap();
            state = next;
            assert!(!wait.timed_out(), "first save did not start");
        }
    }

    fn release_first_save(&self) {
        self.shared.state.lock().unwrap().release_first = true;
        self.shared.changed.notify_all();
    }

    fn saved_notes(&self) -> Vec<String> {
        self.shared.state.lock().unwrap().saved_notes.clone()
    }
}

impl NotesRepository for BlockingRepository {
    fn load(&self) -> Result<NotesDocument, NotesError> {
        Ok(self.shared.state.lock().unwrap().current.clone())
    }

    fn save(&self, document: &NotesDocument) -> Result<(), NotesError> {
        let mut state = self.shared.state.lock().unwrap();
        state.saved_notes.push(document.note.clone());
        if !state.first_started {
            state.first_started = true;
            self.shared.changed.notify_all();
            while !state.release_first {
                state = self.shared.changed.wait(state).unwrap();
            }
        }
        state.current = document.clone();
        Ok(())
    }
}
