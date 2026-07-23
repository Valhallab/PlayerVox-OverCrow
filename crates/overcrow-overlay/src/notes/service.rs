use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
};

use super::{NotesDocument, NotesError, NotesRepository};

pub const NOTES_ERROR_MAX_CHARS: usize = 180;
const WORKER_THREAD_NAME: &str = "overcrow-notes-provider";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotesCommand {
    SetNote(String),
    AddItem(String),
    SetItemText { id: String, text: String },
    SetChecked { id: String, checked: bool },
    RemoveItem { id: String },
}

impl NotesCommand {
    fn apply(self, document: &mut NotesDocument) -> Result<(), NotesError> {
        match self {
            Self::SetNote(note) => document.set_note(note),
            Self::AddItem(text) => document.add_item(text).map(|_| ()),
            Self::SetItemText { id, text } => document.set_item_text(&id, text),
            Self::SetChecked { id, checked } => document.set_checked(&id, checked),
            Self::RemoveItem { id } => document.remove_item(&id),
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
}

impl Default for WorkerState {
    fn default() -> Self {
        let document = NotesDocument::default();
        Self {
            ready: false,
            committed: document.clone(),
            desired: document,
            pending: None,
        }
    }
}

#[derive(Clone)]
struct UpdatePublisher {
    slot: Arc<Mutex<Option<NotesUpdate>>>,
    ready: SyncSender<()>,
}

struct UpdateReceiver {
    slot: Arc<Mutex<Option<NotesUpdate>>>,
    ready: Receiver<()>,
}

fn update_channel() -> (UpdatePublisher, UpdateReceiver) {
    let slot = Arc::new(Mutex::new(None));
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
    fn publish(&self, update: NotesUpdate) -> bool {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(update);
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
            .take()
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
    pub fn spawn(
        repository: impl NotesRepository,
        request_repaint: impl Fn() + Send + 'static,
    ) -> Self {
        let state = Arc::new(Mutex::new(WorkerState::default()));
        let (wake, wake_receiver) = mpsc::sync_channel(1);
        let (publisher, updates) = update_channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_publisher = publisher.clone();
        let worker_shutdown = Arc::clone(&shutdown);
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
                );
            })
            .ok();

        if worker.is_none() {
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.ready = true;
            let update = update_for_state(
                &state,
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
        let update = update_for_state(&state, None, false);
        drop(state);
        self.publisher.publish(update);
        match self.wake.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => Ok(()),
            Err(TrySendError::Disconnected(())) => {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.desired = state.committed.clone();
                state.pending = None;
                let update = update_for_state(
                    &state,
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
        self.shutdown.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
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
) {
    let load_result = repository.load();
    let initial_update = {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.ready = true;
        match load_result {
            Ok(document) => {
                state.committed = document.clone();
                state.desired = document;
                update_for_state(&state, None, false)
            }
            Err(error) => update_for_state(&state, Some(error.to_string()), false),
        }
    };
    publish(&publisher, initial_update, &request_repaint);

    while wake.recv().is_ok() {
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        loop {
            let candidate = {
                state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pending
                    .take()
            };
            let Some(candidate) = candidate else {
                break;
            };

            let result = repository.save(&candidate);
            let update = {
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                match result {
                    Ok(()) => {
                        state.committed = candidate.clone();
                        if state.pending.is_none() {
                            state.desired = candidate;
                        }
                        update_for_state(&state, None, false)
                    }
                    Err(error) if error.was_committed() => {
                        state.committed = candidate.clone();
                        if state.pending.is_none() {
                            state.desired = candidate;
                        }
                        update_for_state(&state, Some(error.to_string()), true)
                    }
                    Err(error) => {
                        if state.pending.is_none() {
                            state.desired = state.committed.clone();
                        }
                        update_for_state(&state, Some(error.to_string()), false)
                    }
                }
            };
            publish(&publisher, update, &request_repaint);

            if shutdown.load(Ordering::Acquire) {
                return;
            }
        }
    }
}

fn publish(publisher: &UpdatePublisher, update: NotesUpdate, request_repaint: &impl Fn()) {
    if publisher.publish(update) {
        request_repaint();
    }
}

fn update_for_state(
    state: &WorkerState,
    error: Option<String>,
    durability_warning: bool,
) -> NotesUpdate {
    NotesUpdate {
        document: state.committed.clone(),
        save_pending: state.pending.is_some(),
        error: error.map(bound_error),
        durability_warning,
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
