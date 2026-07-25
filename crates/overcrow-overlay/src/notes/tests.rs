use std::{
    ffi::{CString, OsStr},
    fs, io,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, Instant},
};

use overcrow_logging::{Component, LoggerRuntime};
use serde_json::json;
use tempfile::NamedTempFile;

use super::{
    ChecklistItem, LocalNotesRepository, NOTES_FILE_MAX_BYTES, NOTES_IDENTIFIER_MAX_BYTES,
    NOTES_ITEM_MAX_BYTES, NOTES_ITEM_MAX_COUNT, NOTES_NOTE_MAX_BYTES, NOTES_PAGE_MAX_COUNT,
    NOTES_SCHEMA_VERSION, NOTES_TITLE_MAX_BYTES, NotePage, NotesCommand, NotesDocument, NotesError,
    NotesProviderRef, NotesRepository, NotesService, NotesUpdate,
    store::{AtomicWriter, notes_path},
};

fn update_general(body: impl Into<String>) -> NotesCommand {
    NotesCommand::UpdateNote {
        id: "note-1".to_owned(),
        title: "General".to_owned(),
        body: body.into(),
    }
}

#[test]
fn default_document_has_one_active_general_page() {
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
    assert_eq!(document.next_note_id, 2);
    assert_eq!(document.next_item_id, 1);
    assert_eq!(document.active_note_id, "note-1");
    assert_eq!(
        document.notes,
        vec![NotePage {
            id: "note-1".to_owned(),
            title: "General".to_owned(),
            body: String::new(),
            items: Vec::new(),
        }]
    );
    assert!(document.validate().is_ok());
}

#[test]
fn note_ids_are_monotonic_and_removing_the_active_page_selects_its_neighbor() {
    let mut document = NotesDocument::default();
    let second = document.add_note("Build").unwrap();
    let third = document.add_note("Farm").unwrap();

    document.remove_note(&third).unwrap();

    assert_eq!(second, "note-2");
    assert_eq!(third, "note-3");
    assert_eq!(document.active_note_id, second);
    assert_eq!(document.next_note_id, 4);
}

#[test]
fn the_last_note_cannot_be_removed() {
    let mut document = NotesDocument::default();
    let committed = document.clone();

    let error = document.remove_note("note-1").unwrap_err();

    assert!(matches!(error, NotesError::Validation(_)));
    assert_eq!(document, committed);
}

#[test]
fn page_and_title_limits_are_atomic() {
    let mut document = NotesDocument::default();
    document
        .update_note("note-1", "a".repeat(NOTES_TITLE_MAX_BYTES), "")
        .unwrap();
    for index in 1..NOTES_PAGE_MAX_COUNT {
        document.add_note(format!("Note {index}")).unwrap();
    }
    let committed = document.clone();

    assert!(matches!(
        document.add_note("one too many"),
        Err(NotesError::Validation(_))
    ));
    assert_eq!(document, committed);

    assert!(matches!(
        document.update_note(
            "note-1",
            format!("{}é", "a".repeat(NOTES_TITLE_MAX_BYTES - 1)),
            ""
        ),
        Err(NotesError::Validation(_))
    ));
    assert_eq!(document, committed);
}

#[test]
fn checklist_item_ids_remain_unique_across_notes() {
    let mut document = NotesDocument::default();
    let first = document.add_item("first").unwrap();
    let second_note = document.add_note("Second").unwrap();
    let second = document.add_item_to(&second_note, "second").unwrap();

    assert_eq!(first, "local-1");
    assert_eq!(second, "local-2");
    assert_eq!(document.next_item_id, 3);
}

#[test]
fn checked_mutation_increments_revision_without_changing_item_identity() {
    let mut document = NotesDocument::default();
    let id = document.add_item("Find the shrine").unwrap();

    document.set_checked(&id, true).unwrap();

    assert_eq!(document.revision, 2);
    let active = document
        .active_note()
        .expect("valid test document has an active note");
    assert_eq!(active.items[0].id, id);
    assert!(active.items[0].checked);
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
    assert_eq!(document.next_item_id, 4);
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
    assert_eq!(
        document
            .active_note()
            .expect("valid test document has an active note")
            .body,
        "plain <b>text</b>"
    );
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
    oversized_item.notes[0].items.push(ChecklistItem {
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
    let invalid_schema = NotesDocument {
        schema_version: NOTES_SCHEMA_VERSION + 1,
        ..NotesDocument::default()
    };
    let invalid_identity = NotesDocument {
        id: "another".to_owned(),
        ..NotesDocument::default()
    };
    let invalid_counter = NotesDocument {
        next_item_id: 0,
        ..NotesDocument::default()
    };
    let mut duplicate_items = NotesDocument {
        next_item_id: 2,
        ..NotesDocument::default()
    };
    duplicate_items.notes[0].items = vec![
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
    ];
    let mut invalid_item_counter = NotesDocument {
        next_item_id: 2,
        ..NotesDocument::default()
    };
    invalid_item_counter.notes[0].items = vec![ChecklistItem {
        id: "local-2".to_owned(),
        text: String::new(),
        checked: false,
    }];
    let unknown_active = NotesDocument {
        active_note_id: "note-99".to_owned(),
        ..NotesDocument::default()
    };

    let invalid_documents = [
        invalid_schema,
        invalid_identity,
        invalid_counter,
        duplicate_items,
        invalid_item_counter,
        unknown_active,
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
    item["notes"][0]["items"] =
        json!([{"id":"local-1","text":"x","checked":false,"unexpected":true}]);
    item["next_item_id"] = json!(2);

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
fn schema_v1_load_migrates_content_without_rewriting_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("global.json");
    let legacy = br#"{
  "schema_version": 1,
  "id": "global",
  "provider": {"kind": "local", "remote_id": null},
  "revision": 7,
  "next_local_id": 2,
  "note": "legacy body",
  "items": [{"id": "local-1", "text": "legacy entry", "checked": true}]
}"#;
    write_private(&path, legacy);

    let document = LocalNotesRepository::from_path(&path).load().unwrap();

    assert_eq!(document.schema_version, NOTES_SCHEMA_VERSION);
    assert_eq!(document.revision, 7);
    assert_eq!(document.next_item_id, 2);
    let active = document
        .active_note()
        .expect("migrated document has an active note");
    assert_eq!(active.title, "General");
    assert_eq!(active.body, "legacy body");
    assert_eq!(active.items[0].text, "legacy entry");
    assert!(active.items[0].checked);
    assert_eq!(fs::read(path).unwrap(), legacy);
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
fn largest_valid_document_fits_the_repository_limit_after_json_escaping() {
    let escaped = "\0".repeat(NOTES_NOTE_MAX_BYTES);
    let item_text = "\0".repeat(NOTES_ITEM_MAX_BYTES);
    let mut document = NotesDocument::default();
    document
        .update_note("note-1", "\0".repeat(NOTES_TITLE_MAX_BYTES), &escaped)
        .unwrap();
    for note_index in 0..NOTES_PAGE_MAX_COUNT {
        let note_id = if note_index == 0 {
            "note-1".to_owned()
        } else {
            let id = document
                .add_note("\0".repeat(NOTES_TITLE_MAX_BYTES))
                .unwrap();
            document
                .update_note(&id, "\0".repeat(NOTES_TITLE_MAX_BYTES), &escaped)
                .unwrap();
            id
        };
        for _ in 0..NOTES_ITEM_MAX_COUNT {
            document.add_item_to(&note_id, &item_text).unwrap();
        }
    }

    document.validate().unwrap();
    let serialized = serde_json::to_vec_pretty(&document).unwrap();

    assert!(
        serialized.len() < NOTES_FILE_MAX_BYTES,
        "maximum valid notes document needs {} bytes",
        serialized.len() + 1
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
    let mut invalid = NotesDocument::default();
    invalid.notes[0].body = "x".repeat(NOTES_NOTE_MAX_BYTES + 1);

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

    service.send(update_general("note")).unwrap();
    let saved = wait_for_settled(&service);
    assert_eq!(
        saved
            .document
            .active_note()
            .expect("valid test document has an active note")
            .body,
        "note"
    );
    let id = saved
        .document
        .active_note()
        .expect("valid test document has an active note")
        .items
        .first()
        .map(|item| item.id.clone());
    assert!(id.is_none());

    service
        .send(NotesCommand::AddItem {
            note_id: "note-1".to_owned(),
            text: "item".to_owned(),
        })
        .unwrap();
    let saved = wait_for_settled(&service);
    let id = saved
        .document
        .active_note()
        .expect("valid test document has an active note")
        .items[0]
        .id
        .clone();
    service
        .send(NotesCommand::SetItemText {
            note_id: "note-1".to_owned(),
            id: id.clone(),
            text: "edited".to_owned(),
        })
        .unwrap();
    wait_for_settled(&service);
    service
        .send(NotesCommand::SetChecked {
            note_id: "note-1".to_owned(),
            id: id.clone(),
            checked: true,
        })
        .unwrap();
    wait_for_settled(&service);
    service
        .send(NotesCommand::RemoveItem {
            note_id: "note-1".to_owned(),
            id,
        })
        .unwrap();
    let saved = wait_for_settled(&service);

    assert!(
        saved
            .document
            .active_note()
            .expect("valid test document has an active note")
            .items
            .is_empty()
    );
    assert_eq!(saved.document.revision, 5);
    assert_eq!(repository.current(), saved.document);
}

#[test]
fn rejected_command_does_not_queue_or_change_state() {
    let repository = MemoryRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository.clone(), || {});
    let ready = wait_for_ready(&service);

    let error = service
        .send(update_general("x".repeat(NOTES_NOTE_MAX_BYTES + 1)))
        .unwrap_err();

    assert!(matches!(error, NotesError::Validation(_)));
    assert_eq!(service.current(), ready.document);
    assert_eq!(repository.save_count(), 0);
}

#[test]
fn disconnected_worker_rejects_without_publishing_a_candidate() {
    let service = NotesService::unavailable_for_tests();
    let committed = service.current();

    let error = service.send(update_general("must not leak")).unwrap_err();
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

    service.send(update_general("must roll back")).unwrap();
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

    service.send(update_general("committed on disk")).unwrap();
    let update = wait_for_error(&service);

    assert_eq!(
        update
            .document
            .active_note()
            .expect("valid test document has an active note")
            .body,
        "committed on disk"
    );
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

    service.send(update_general("first")).unwrap();
    repository.wait_for_first_save();
    service.send(update_general("intermediate")).unwrap();
    service.send(update_general("latest")).unwrap();
    repository.release_first_save();
    let settled = wait_for_settled(&service);

    assert_eq!(
        settled
            .document
            .active_note()
            .expect("valid test document has an active note")
            .body,
        "latest"
    );
    assert_eq!(
        repository.saved_notes(),
        ["first".to_owned(), "latest".to_owned()]
    );
}

#[test]
fn shutdown_drains_the_latest_accepted_candidate() {
    let repository = BlockingRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository.clone(), || {});
    wait_for_ready(&service);

    service.send(update_general("first")).unwrap();
    repository.wait_for_first_save();
    service.send(update_general("latest")).unwrap();
    service.begin_shutdown_for_tests();
    repository.release_first_save();
    drop(service);

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

    service.send(update_general("not committed yet")).unwrap();
    repository.wait_for_first_save();
    let pending = wait_for_update(&service, |update| update.save_pending);
    let current_while_pending = service.current();
    repository.release_first_save();
    let settled = wait_for_settled(&service);

    assert_eq!(pending.document, committed);
    assert_eq!(current_while_pending, committed);
    assert_eq!(
        settled
            .document
            .active_note()
            .expect("valid test document has an active note")
            .body,
        "not committed yet"
    );
}

#[test]
fn latest_result_publication_coalesces_pending_updates() {
    let repository = MemoryRepository::new(NotesDocument::default());
    let service = NotesService::spawn(repository, || {});
    wait_for_ready(&service);

    service.send(update_general("one")).unwrap();
    service.send(update_general("two")).unwrap();
    let update = wait_for_settled(&service);

    assert_eq!(
        update
            .document
            .active_note()
            .expect("valid test document has an active note")
            .body,
        "two"
    );
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
fn notes_diagnostics_omit_private_content_and_report_recovery() {
    let temp = tempfile::tempdir().expect("create log directory");
    let log_runtime =
        LoggerRuntime::start_in(Component::Overlay, temp.path()).expect("start test logger");
    let repository = MemoryRepository::new(NotesDocument::default());
    let service = NotesService::spawn_with_logger(repository.clone(), log_runtime.logger(), || {});
    wait_for_ready(&service);
    repository.fail_next(false);

    service
        .send(update_general("private note content"))
        .unwrap();
    wait_for_error(&service);
    service
        .send(update_general("recovered private content"))
        .unwrap();
    wait_for_settled(&service);
    drop(service);
    drop(log_runtime);

    let contents =
        fs::read_to_string(temp.path().join("overlay.log")).expect("read diagnostic log");
    assert_eq!(contents.matches("widget_provider_failed").count(), 1);
    assert!(contents.contains("widget=notes provider=local_notes category=filesystem"));
    assert!(contents.contains("widget_provider_recovered widget=notes provider=local_notes"));
    assert!(!contents.contains("forced save failure"));
    assert!(!contents.contains("private note content"));
    assert!(!contents.contains("recovered private content"));
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
        state.saved_notes.push(
            document
                .active_note()
                .expect("repository only receives validated documents")
                .body
                .clone(),
        );
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
