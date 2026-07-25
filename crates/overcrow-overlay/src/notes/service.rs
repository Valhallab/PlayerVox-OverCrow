use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
};

use overcrow_logging::EventLogger;

use crate::runtime::widget_diagnostics::{FailureCategory, Provider, ProviderDiagnostics};

use super::{NotesDocument, NotesError, NotesRepository};

pub const NOTES_ERROR_MAX_CHARS: usize = 180;
const WORKER_THREAD_NAME: &str = "overcrow-notes-provider";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotesCommand {
    AddNote {
        title: String,
    },
    SetActiveNote {
        id: String,
    },
    UpdateNote {
        id: String,
        title: String,
        body: String,
    },
    RemoveNote {
        id: String,
    },
    AddItem {
        note_id: String,
        text: String,
    },
    SetItemText {
        note_id: String,
        id: String,
        text: String,
    },
    SetChecked {
        note_id: String,
        id: String,
        checked: bool,
    },
    RemoveItem {
        note_id: String,
        id: String,
    },
}

impl NotesCommand {
    pub(crate) fn apply(&self, document: &mut NotesDocument) -> Result<(), NotesError> {
        match self {
            Self::AddNote { title } => document.add_note(title.clone()).map(|_| ()),
            Self::SetActiveNote { id } => document.set_active_note(id),
            Self::UpdateNote { id, title, body } => {
                document.update_note(id, title.clone(), body.clone())
            }
            Self::RemoveNote { id } => document.remove_note(id),
            Self::AddItem { note_id, text } => {
                document.add_item_to(note_id, text.clone()).map(|_| ())
            }
            Self::SetItemText { note_id, id, text } => {
                document.set_item_text_in(note_id, id, text.clone())
            }
            Self::SetChecked {
                note_id,
                id,
                checked,
            } => document.set_checked_in(note_id, id, *checked),
            Self::RemoveItem { note_id, id } => document.remove_item_from(note_id, id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotesUpdate {
    pub document: NotesDocument,
    pub save_pending: bool,
    pub error: Option<String>,
    pub durability_warning: bool,
}

struct WorkerState {
    ready: bool,
    committed: NotesDocument,
    desired: NotesDocument,
    pending: Option<NotesDocument>,
    update_generation: u64,
}

impl Default for WorkerState {
    fn default() -> Self {
        let document = NotesDocument::default();
        Self {
            ready: false,
            committed: document.clone(),
            desired: document,
            pending: None,
            update_generation: 0,
        }
    }
}

struct PublishedNotesUpdate {
    generation: u64,
    update: NotesUpdate,
}

#[derive(Default)]
struct UpdateSlot {
    newest_generation: u64,
    latest: Option<PublishedNotesUpdate>,
}

#[derive(Clone)]
struct UpdatePublisher {
    slot: Arc<Mutex<UpdateSlot>>,
    ready: SyncSender<()>,
}

struct UpdateReceiver {
    slot: Arc<Mutex<UpdateSlot>>,
    ready: Receiver<()>,
}

fn update_channel() -> (UpdatePublisher, UpdateReceiver) {
    let slot = Arc::new(Mutex::new(UpdateSlot::default()));
    let (ready, receiver) = mpsc::sync_channel(1);
    (
        UpdatePublisher {
            slot: Arc::clone(&slot),
            ready,
        },
        UpdateReceiver {
            slot,
            ready: receiver,
        },
    )
}

impl UpdatePublisher {
    fn publish(&self, update: PublishedNotesUpdate) -> bool {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if update.generation <= slot.newest_generation {
            return false;
        }
        slot.newest_generation = update.generation;
        slot.latest = Some(update);
        drop(slot);
        match self.ready.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => true,
            Err(TrySendError::Disconnected(())) => false,
        }
    }
}

impl UpdateReceiver {
    fn take_latest(&self) -> Option<NotesUpdate> {
        self.ready.try_recv().ok()?;
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .latest
            .take()
            .map(|published| published.update)
    }
}

pub struct NotesService {
    state: Arc<Mutex<WorkerState>>,
    wake: SyncSender<()>,
    updates: UpdateReceiver,
    publisher: UpdatePublisher,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl NotesService {
    #[cfg(test)]
    pub fn spawn(
        repository: impl NotesRepository,
        request_repaint: impl Fn() + Send + 'static,
    ) -> Self {
        Self::spawn_with_logger(repository, EventLogger::disabled(), request_repaint)
    }

    pub fn spawn_with_logger(
        repository: impl NotesRepository,
        logger: EventLogger,
        request_repaint: impl Fn() + Send + 'static,
    ) -> Self {
        let state = Arc::new(Mutex::new(WorkerState::default()));
        let (wake, wake_receiver) = mpsc::sync_channel(1);
        let (publisher, updates) = update_channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_publisher = publisher.clone();
        let worker_shutdown = Arc::clone(&shutdown);
        let spawn_logger = logger.clone();
        let worker = thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_owned())
            .spawn(move || {
                run_worker(
                    repository,
                    worker_state,
                    wake_receiver,
                    worker_publisher,
                    worker_shutdown,
                    request_repaint,
                    ProviderDiagnostics::new(logger, Provider::LocalNotes),
                );
            })
            .inspect_err(|_| {
                ProviderDiagnostics::new(spawn_logger, Provider::LocalNotes)
                    .failed(FailureCategory::Startup);
            })
            .ok();

        if worker.is_none() {
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.ready = true;
            let update = next_update_for_state(
                &mut state,
                Some("notes worker could not be started".to_owned()),
                false,
            );
            drop(state);
            publisher.publish(update);
        }

        Self {
            state,
            wake,
            updates,
            publisher,
            shutdown,
            worker,
        }
    }

    pub fn send(&self, command: NotesCommand) -> Result<(), NotesError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.ready {
            return Err(NotesError::repository("notes are still loading"));
        }
        let mut candidate = state.desired.clone();
        command.apply(&mut candidate)?;
        candidate.validate()?;
        state.desired = candidate.clone();
        state.pending = Some(candidate);
        let update = next_update_for_state(&mut state, None, false);
        drop(state);
        self.publisher.publish(update);
        match self.wake.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => Ok(()),
            Err(TrySendError::Disconnected(())) => {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.desired = state.committed.clone();
                state.pending = None;
                let update = next_update_for_state(
                    &mut state,
                    Some("notes repository failed: notes worker unavailable".to_owned()),
                    false,
                );
                drop(state);
                self.publisher.publish(update);
                Err(NotesError::repository("notes worker unavailable"))
            }
        }
    }

    #[cfg(test)]
    pub(super) fn unavailable_for_tests() -> Self {
        let worker_state = WorkerState {
            ready: true,
            ..WorkerState::default()
        };
        let state = Arc::new(Mutex::new(worker_state));
        let (wake, wake_receiver) = mpsc::sync_channel(1);
        drop(wake_receiver);
        let (publisher, updates) = update_channel();
        Self {
            state,
            wake,
            updates,
            publisher,
            shutdown: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }

    pub fn take_latest(&self) -> Option<NotesUpdate> {
        self.updates.take_latest()
    }

    fn begin_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
    }

    #[cfg(test)]
    pub(super) fn begin_shutdown_for_tests(&self) {
        self.begin_shutdown();
    }

    #[cfg(test)]
    pub fn current(&self) -> NotesDocument {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .committed
            .clone()
    }
}

impl Drop for NotesService {
    fn drop(&mut self) {
        self.begin_shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    repository: impl NotesRepository,
    state: Arc<Mutex<WorkerState>>,
    wake: Receiver<()>,
    publisher: UpdatePublisher,
    shutdown: Arc<AtomicBool>,
    request_repaint: impl Fn(),
    mut diagnostics: ProviderDiagnostics,
) {
    let load_result = repository.load();
    match &load_result {
        Ok(_) => diagnostics.recovered(),
        Err(error) => diagnostics.failed(notes_failure_category(error)),
    }
    let initial_update = {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.ready = true;
        match load_result {
            Ok(document) => {
                state.committed = document.clone();
                state.desired = document;
                next_update_for_state(&mut state, None, false)
            }
            Err(error) => next_update_for_state(&mut state, Some(error.to_string()), false),
        }
    };
    publish(&publisher, initial_update, &request_repaint);

    while wake.recv().is_ok() {
        loop {
            let candidate = {
                state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pending
                    .take()
            };
            let Some(candidate) = candidate else {
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                break;
            };

            let result = repository.save(&candidate);
            match &result {
                Ok(()) => diagnostics.recovered(),
                Err(error) => diagnostics.failed(notes_failure_category(error)),
            }
            let update = {
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                match result {
                    Ok(()) => {
                        state.committed = candidate.clone();
                        if state.pending.is_none() {
                            state.desired = candidate;
                        }
                        next_update_for_state(&mut state, None, false)
                    }
                    Err(error) if error.was_committed() => {
                        state.committed = candidate.clone();
                        if state.pending.is_none() {
                            state.desired = candidate;
                        }
                        next_update_for_state(&mut state, Some(error.to_string()), true)
                    }
                    Err(error) => {
                        if state.pending.is_none() {
                            state.desired = state.committed.clone();
                        }
                        next_update_for_state(&mut state, Some(error.to_string()), false)
                    }
                }
            };
            publish(&publisher, update, &request_repaint);
        }
    }
}

fn notes_failure_category(error: &NotesError) -> FailureCategory {
    match error {
        NotesError::Validation(_) => FailureCategory::Validation,
        NotesError::Json(_) => FailureCategory::Parse,
        NotesError::Io(_) | NotesError::Repository(_) => FailureCategory::Filesystem,
        NotesError::Committed(_) => FailureCategory::Durability,
    }
}

fn publish(publisher: &UpdatePublisher, update: PublishedNotesUpdate, request_repaint: &impl Fn()) {
    if publisher.publish(update) {
        request_repaint();
    }
}

fn next_update_for_state(
    state: &mut WorkerState,
    error: Option<String>,
    durability_warning: bool,
) -> PublishedNotesUpdate {
    state.update_generation = state.update_generation.saturating_add(1);
    PublishedNotesUpdate {
        generation: state.update_generation,
        update: NotesUpdate {
            document: state.committed.clone(),
            save_pending: state.pending.is_some(),
            error: error.map(bound_error),
            durability_warning,
        },
    }
}

fn bound_error(message: String) -> String {
    if message.chars().count() <= NOTES_ERROR_MAX_CHARS {
        return message;
    }
    let mut bounded = message
        .chars()
        .take(NOTES_ERROR_MAX_CHARS.saturating_sub(1))
        .collect::<String>();
    bounded.push('…');
    bounded
}

#[cfg(test)]
mod publication_tests {
    use super::{NotesDocument, NotesUpdate, PublishedNotesUpdate, update_channel};

    fn published(generation: u64, note: &str) -> PublishedNotesUpdate {
        let mut document = NotesDocument::default();
        document.set_note(note).expect("valid test note");
        PublishedNotesUpdate {
            generation,
            update: NotesUpdate {
                document,
                save_pending: generation == 2,
                error: None,
                durability_warning: false,
            },
        }
    }

    #[test]
    fn stale_publication_cannot_replace_a_newer_pending_update() {
        let (publisher, receiver) = update_channel();

        assert!(publisher.publish(published(2, "newer")));
        let latest = receiver.take_latest().expect("latest update");
        assert_eq!(
            latest
                .document
                .active_note()
                .expect("valid test document has an active note")
                .body,
            "newer"
        );
        assert!(latest.save_pending);

        assert!(!publisher.publish(published(1, "stale")));
        assert!(receiver.take_latest().is_none());
    }
}
