use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use overcrow_protocol::VersionedCoreSnapshot;
use tokio::sync::watch;

use crate::CoreService;

pub type SignalFuture<'a> = Pin<Box<dyn Future<Output = zbus::Result<()>> + Send + 'a>>;

pub trait SnapshotSignalSink: Send + Sync {
    fn emit<'a>(&'a self, snapshot_json: &'a str) -> SignalFuture<'a>;
}

pub struct DbusSnapshotSignalSink {
    emitter: zbus::object_server::SignalEmitter<'static>,
}

impl DbusSnapshotSignalSink {
    pub fn new(emitter: zbus::object_server::SignalEmitter<'static>) -> Self {
        Self { emitter }
    }
}

impl SnapshotSignalSink for DbusSnapshotSignalSink {
    fn emit<'a>(&'a self, snapshot_json: &'a str) -> SignalFuture<'a> {
        Box::pin(CoreService::snapshot_changed(&self.emitter, snapshot_json))
    }
}

pub async fn run_snapshot_signal_publisher<S>(
    mut receiver: watch::Receiver<VersionedCoreSnapshot>,
    sink: Arc<S>,
) where
    S: SnapshotSignalSink + ?Sized,
{
    while receiver.changed().await.is_ok() {
        let snapshot = receiver.borrow_and_update().clone();
        let snapshot_json = match CoreService::versioned_snapshot_json(&snapshot) {
            Ok(snapshot_json) => snapshot_json,
            Err(error) => {
                log_bounded_error("serialization", &error);
                continue;
            }
        };
        if let Err(error) = sink.emit(&snapshot_json).await {
            log_bounded_error("emission", &error);
        }
    }
}

fn log_bounded_error(context: &str, error: &dyn std::fmt::Display) {
    let message: String = error.to_string().chars().take(512).collect();
    eprintln!("OverCrow snapshot signal {context} failed: {message}");
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use overcrow_config::LifecycleSettings;
    use overcrow_protocol::{CoreState, GameWindow, Rect, VersionedCoreSnapshot};
    use tokio::sync::{Notify, RwLock};

    use super::{SignalFuture, SnapshotSignalSink, run_snapshot_signal_publisher};
    use crate::{CoreRuntime, ProcessInfo};

    #[derive(Default)]
    struct RecordingSink {
        messages: Mutex<Vec<String>>,
        attempts: AtomicUsize,
        delay_next: AtomicBool,
        fail_next: AtomicBool,
        changed: Notify,
        release: Notify,
    }

    impl RecordingSink {
        fn failing_once() -> Self {
            Self {
                fail_next: AtomicBool::new(true),
                ..Self::default()
            }
        }

        fn delaying_once() -> Self {
            Self {
                delay_next: AtomicBool::new(true),
                ..Self::default()
            }
        }

        fn release_delayed_emission(&self) {
            self.release.notify_one();
        }

        async fn wait_for_attempts(&self, expected: usize) {
            loop {
                let changed = self.changed.notified();
                if self.attempts.load(Ordering::SeqCst) >= expected {
                    return;
                }
                changed.await;
            }
        }

        async fn wait_for_len(&self, expected: usize) {
            loop {
                let changed = self.changed.notified();
                if self.messages.lock().expect("recorded messages").len() >= expected {
                    return;
                }
                changed.await;
            }
        }

        fn decoded(&self) -> Vec<VersionedCoreSnapshot> {
            self.messages
                .lock()
                .expect("recorded messages")
                .iter()
                .map(|json| serde_json::from_str(json).expect("versioned snapshot JSON"))
                .collect()
        }
    }

    impl SnapshotSignalSink for RecordingSink {
        fn emit<'a>(&'a self, snapshot_json: &'a str) -> SignalFuture<'a> {
            Box::pin(async move {
                self.attempts.fetch_add(1, Ordering::SeqCst);
                self.changed.notify_waiters();
                if self.delay_next.swap(false, Ordering::SeqCst) {
                    self.release.notified().await;
                }
                if self.fail_next.swap(false, Ordering::SeqCst) {
                    return Err(zbus::Error::Failure("transport failed".repeat(128)));
                }
                self.messages
                    .lock()
                    .expect("recorded messages")
                    .push(snapshot_json.to_owned());
                self.changed.notify_waiters();
                Ok(())
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn snapshot_signal_delivers_observed_changes_and_suppresses_equal_state() {
        let runtime = active_runtime().await;
        let sink = Arc::new(RecordingSink::default());
        let publisher = tokio::spawn(run_snapshot_signal_publisher(
            runtime.snapshots(),
            Arc::clone(&sink),
        ));

        runtime.set_overlay_interactive(true).await;
        sink.wait_for_len(1).await;
        runtime.set_overlay_interactive(true).await;
        tokio::task::yield_now().await;
        assert_eq!(sink.decoded().len(), 1);
        runtime.set_overlay_interactive(false).await;
        sink.wait_for_len(2).await;

        let messages = sink.decoded();
        assert_eq!(messages.len(), 2);
        assert!(messages[0].revision < messages[1].revision);

        publisher.abort();
        let _ = publisher.await;
    }

    #[tokio::test(start_paused = true)]
    async fn snapshot_signal_emits_a_change_published_after_subscription_before_start() {
        let runtime = active_runtime().await;
        let receiver = runtime.snapshots();
        let sink = Arc::new(RecordingSink::default());

        runtime.set_overlay_interactive(true).await;
        let expected = runtime.versioned_snapshot();
        assert!(
            receiver
                .has_changed()
                .expect("snapshot sender remains open")
        );

        let publisher = tokio::spawn(run_snapshot_signal_publisher(receiver, Arc::clone(&sink)));
        sink.wait_for_len(1).await;

        assert_eq!(sink.decoded(), vec![expected]);

        publisher.abort();
        let _ = publisher.await;
    }

    #[tokio::test(start_paused = true)]
    async fn snapshot_signal_continues_after_one_emission_failure() {
        let runtime = active_runtime().await;
        let sink = Arc::new(RecordingSink::failing_once());
        let publisher = tokio::spawn(run_snapshot_signal_publisher(
            runtime.snapshots(),
            Arc::clone(&sink),
        ));

        runtime.set_overlay_interactive(true).await;
        sink.wait_for_attempts(1).await;
        runtime.set_overlay_interactive(false).await;
        sink.wait_for_len(1).await;

        assert_eq!(sink.attempts.load(Ordering::SeqCst), 2);
        assert_eq!(sink.decoded(), vec![runtime.versioned_snapshot()]);
        assert!(!publisher.is_finished());

        publisher.abort();
        let _ = publisher.await;
    }

    #[tokio::test(start_paused = true)]
    async fn delayed_snapshot_signal_converges_to_the_newest_revision() {
        let runtime = active_runtime().await;
        let sink = Arc::new(RecordingSink::delaying_once());
        let publisher = tokio::spawn(run_snapshot_signal_publisher(
            runtime.snapshots(),
            Arc::clone(&sink),
        ));

        runtime.set_overlay_interactive(true).await;
        sink.wait_for_attempts(1).await;
        runtime.set_overlay_interactive(false).await;
        runtime.set_overlay_interactive(true).await;
        let newest = runtime.versioned_snapshot();
        sink.release_delayed_emission();
        sink.wait_for_len(2).await;

        let messages = sink.decoded();
        assert_eq!(messages[1], newest);
        assert!(messages[0].revision + 1 < messages[1].revision);

        publisher.abort();
        let _ = publisher.await;
    }

    async fn active_runtime() -> CoreRuntime {
        let mut state = CoreState::default();
        state.observe_game(sample_game());
        CoreRuntime::with_settings(
            Arc::new(RwLock::new(state)),
            HashMap::from([(42, sample_process())]),
            LifecycleSettings {
                enabled: true,
                selected_steam_app_ids: BTreeSet::from([620]),
                ..LifecycleSettings::default()
            },
        )
        .await
    }

    fn sample_game() -> GameWindow {
        GameWindow {
            pid: Some(42),
            steam_app_id: Some(620),
            app_id: Some("portal2".to_owned()),
            title: "Portal 2".to_owned(),
            rect: Rect {
                x: 100,
                y: 200,
                width: 1920,
                height: 1080,
            },
            scale: 1.0,
            backend: "x11".to_owned(),
        }
    }

    fn sample_process() -> ProcessInfo {
        ProcessInfo {
            pid: 42,
            parent_pid: 1,
            start_ticks: 0,
            timing: None,
            resources: Default::default(),
            name: "portal2".to_owned(),
            environment: HashMap::from([("SteamAppId".to_owned(), "620".to_owned())]),
            command_line: vec!["portal2".to_owned()],
            executable: Some(PathBuf::from("/games/portal2")),
        }
    }
}
