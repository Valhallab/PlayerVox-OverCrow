use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
};

use overcrow_config::{TwitchPrefs, TwitchPrefsStore, settings_save_was_committed};

use crate::runtime::{LatestReceiver, VersionedValue, latest_channel};

const SAVE_QUEUE_CAPACITY: usize = 1;
const WORKER_THREAD_NAME: &str = "overcrow-twitch-settings";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TwitchPrefsSaveFailure {
    Validation,
    Busy,
    Filesystem,
    Unavailable,
}

impl TwitchPrefsSaveFailure {
    pub const fn category(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Busy | Self::Unavailable => "command",
            Self::Filesystem => "filesystem",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TwitchPrefsSaveOutcome {
    Durable(TwitchPrefs),
    CommittedWithWarning(TwitchPrefs),
    RolledBack(TwitchPrefsSaveFailure),
}

pub struct TwitchPrefsSaver {
    commands: Option<SyncSender<TwitchPrefs>>,
    results: LatestReceiver<TwitchPrefsSaveOutcome>,
    busy: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl TwitchPrefsSaver {
    pub fn spawn(
        store: TwitchPrefsStore,
        request_repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (commands, receiver) = mpsc::sync_channel(SAVE_QUEUE_CAPACITY);
        let (publisher, results) = latest_channel(TwitchPrefsSaveOutcome::RolledBack(
            TwitchPrefsSaveFailure::Unavailable,
        ));
        let busy = Arc::new(AtomicBool::new(false));
        let worker = thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_owned())
            .spawn(move || {
                while let Ok(candidate) = receiver.recv() {
                    let outcome = save_candidate(candidate, |candidate| store.save(candidate));
                    if publisher.publish(outcome) {
                        request_repaint();
                    }
                }
            })
            .ok();
        Self {
            commands: Some(commands),
            results,
            busy,
            worker,
        }
    }

    pub fn try_save(&self, candidate: TwitchPrefs) -> Result<(), TwitchPrefsSaveFailure> {
        let candidate = candidate
            .validate()
            .map_err(|_| TwitchPrefsSaveFailure::Validation)?;
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TwitchPrefsSaveFailure::Busy);
        }
        let result = self
            .commands
            .as_ref()
            .ok_or(TwitchPrefsSaveFailure::Unavailable)?
            .try_send(candidate);
        match result {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.busy.store(false, Ordering::Release);
                Err(TwitchPrefsSaveFailure::Unavailable)
            }
        }
    }

    pub fn take_latest(&self) -> Option<VersionedValue<TwitchPrefsSaveOutcome>> {
        let result = self.results.take_latest();
        if result.is_some() {
            self.busy.store(false, Ordering::Release);
        }
        result
    }
}

impl Drop for TwitchPrefsSaver {
    fn drop(&mut self) {
        self.commands.take();
        // A filesystem sync may still be completing. Detach the bounded worker
        // instead of making renderer shutdown wait on storage.
        self.worker.take();
    }
}

fn save_candidate(
    candidate: TwitchPrefs,
    save: impl FnOnce(&TwitchPrefs) -> io::Result<()>,
) -> TwitchPrefsSaveOutcome {
    match save(&candidate) {
        Ok(()) => TwitchPrefsSaveOutcome::Durable(candidate),
        Err(error) if settings_save_was_committed(&error) => {
            TwitchPrefsSaveOutcome::CommittedWithWarning(candidate)
        }
        Err(_) => TwitchPrefsSaveOutcome::RolledBack(TwitchPrefsSaveFailure::Filesystem),
    }
}

#[cfg(test)]
mod tests {
    use std::{io, time::Duration};

    use overcrow_config::{CommittedSettingsSaveError, TwitchPrefs, TwitchPrefsStore};

    use super::{TwitchPrefsSaveFailure, TwitchPrefsSaveOutcome, TwitchPrefsSaver, save_candidate};

    #[test]
    fn committed_storage_result_carries_the_exact_validated_candidate() {
        let candidate = TwitchPrefs {
            active_channel: Some("warframe".to_owned()),
            ..TwitchPrefs::default()
        };

        assert_eq!(
            save_candidate(candidate.clone(), |_| {
                Err(io::Error::other(CommittedSettingsSaveError::new(
                    io::Error::other("parent sync failed"),
                )))
            }),
            TwitchPrefsSaveOutcome::CommittedWithWarning(candidate)
        );
    }

    #[test]
    fn invalid_candidate_is_rejected_before_the_worker_queue() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let saver = TwitchPrefsSaver::spawn(
            TwitchPrefsStore::from_path(temp.path().join("twitch.json")),
            || {},
        );
        let invalid = TwitchPrefs {
            passive_lifetime_secs: 0,
            ..TwitchPrefs::default()
        };

        assert_eq!(
            saver.try_save(invalid),
            Err(TwitchPrefsSaveFailure::Validation)
        );
    }

    #[test]
    fn save_runs_off_thread_and_publishes_the_durable_candidate() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("overcrow/twitch.json");
        let saver = TwitchPrefsSaver::spawn(TwitchPrefsStore::from_path(&path), || {});
        let candidate = TwitchPrefs {
            active_channel: Some("warframe".to_owned()),
            ..TwitchPrefs::default()
        };
        saver.try_save(candidate.clone()).expect("save queued");

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = saver.take_latest() {
                assert_eq!(
                    result.value.as_ref(),
                    &TwitchPrefsSaveOutcome::Durable(candidate)
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for save"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(path.is_file());
    }
}
