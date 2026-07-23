use std::sync::{
    Arc, Mutex, PoisonError,
    mpsc::{self, Receiver, SyncSender, TrySendError},
};

pub struct VersionedValue<T> {
    pub revision: u64,
    pub value: Arc<T>,
}

impl<T> Clone for VersionedValue<T> {
    fn clone(&self) -> Self {
        Self {
            revision: self.revision,
            value: Arc::clone(&self.value),
        }
    }
}

struct LatestState<T> {
    current: Arc<T>,
    revision: u64,
    pending: bool,
}

pub struct LatestPublisher<T> {
    state: Arc<Mutex<LatestState<T>>>,
    ready: SyncSender<()>,
}

impl<T> Clone for LatestPublisher<T> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            ready: self.ready.clone(),
        }
    }
}

pub struct LatestReceiver<T> {
    state: Arc<Mutex<LatestState<T>>>,
    ready: Receiver<()>,
}

pub fn latest_channel<T>(initial: T) -> (LatestPublisher<T>, LatestReceiver<T>) {
    let state = Arc::new(Mutex::new(LatestState {
        current: Arc::new(initial),
        revision: 0,
        pending: false,
    }));
    let (ready, receiver) = mpsc::sync_channel(1);
    (
        LatestPublisher {
            state: Arc::clone(&state),
            ready,
        },
        LatestReceiver {
            state,
            ready: receiver,
        },
    )
}

impl<T> LatestPublisher<T> {
    pub fn publish(&self, value: T) -> bool {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.revision = state
                .revision
                .checked_add(1)
                .expect("latest-value revision exhausted");
            state.current = Arc::new(value);
            state.pending = true;
        }
        self.notify_receiver()
    }

    pub fn current(&self) -> VersionedValue<T> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        VersionedValue {
            revision: state.revision,
            value: Arc::clone(&state.current),
        }
    }

    fn notify_receiver(&self) -> bool {
        match self.ready.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => true,
            Err(TrySendError::Disconnected(())) => false,
        }
    }
}

impl<T: Clone> LatestPublisher<T> {
    pub fn update(&self, update: impl FnOnce(&mut T)) -> bool {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let mut value = state.current.as_ref().clone();
            update(&mut value);
            state.revision = state
                .revision
                .checked_add(1)
                .expect("latest-value revision exhausted");
            state.current = Arc::new(value);
            state.pending = true;
        }
        self.notify_receiver()
    }
}

impl<T> LatestReceiver<T> {
    pub fn take_latest(&self) -> Option<VersionedValue<T>> {
        self.ready.try_recv().ok()?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.pending {
            return None;
        }
        state.pending = false;
        Some(VersionedValue {
            revision: state.revision,
            value: Arc::clone(&state.current),
        })
    }
}
