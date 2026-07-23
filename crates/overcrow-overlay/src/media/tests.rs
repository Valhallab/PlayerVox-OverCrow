use std::{
    collections::HashMap,
    future,
    sync::mpsc,
    time::{Duration, Instant},
};

use overcrow_logging::{Component, LoggerRuntime};
use overcrow_protocol::OverlayMode;
use zbus::zvariant::{OwnedValue, Str, Value};

use super::{
    client::{
        BackendFuture, Backoff, COMMAND_CAPACITY, INITIAL_BACKOFF, MAXIMUM_BACKOFF, MediaBackend,
        MediaClient, WORKER_THREAD_NAME, WorkerTiming, build_runtime, command_channel,
        publish_if_changed, publish_spawn_failure, snapshot_channel,
    },
    model::{
        MEDIA_ERROR_MAX_BYTES, MediaAction, MediaCapabilities, MediaCommand, MediaPlaybackStatus,
        MediaSnapshot,
    },
    mpris::{
        MPRIS_CANDIDATE_LIMIT, PLAYER_INTERFACE, PLAYER_PATH, TITLE_MAX_BYTES, bound_text,
        command_method, consider_player, filter_mpris_bus_names, parse_metadata,
        parse_playback_status, select_player,
    },
};
use crate::widgets::{MediaControl, MediaPresentation};

fn snapshot(bus_name: &str, status: MediaPlaybackStatus) -> MediaSnapshot {
    MediaSnapshot {
        bus_name: Some(bus_name.to_owned()),
        title: Some("Song".to_owned()),
        artist: Some("Artist".to_owned()),
        playback_status: status,
        capabilities: MediaCapabilities::default(),
        error: None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StalledOperation {
    Connect,
    Discover,
    Command,
}

#[derive(Debug)]
struct BackendEvent {
    operation: &'static str,
    thread_name: Option<String>,
}

struct StallingBackend {
    stalled: StalledOperation,
    events: mpsc::SyncSender<BackendEvent>,
    dropped: Option<mpsc::SyncSender<()>>,
    current: MediaSnapshot,
}

impl StallingBackend {
    fn record(&self, operation: &'static str) {
        self.events
            .send(BackendEvent {
                operation,
                thread_name: std::thread::current().name().map(str::to_owned),
            })
            .unwrap();
    }
}

impl Drop for StallingBackend {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

impl MediaBackend for StallingBackend {
    fn connect(&mut self) -> BackendFuture<'_, ()> {
        self.record("connect");
        if self.stalled == StalledOperation::Connect {
            Box::pin(future::pending())
        } else {
            Box::pin(async { Ok(()) })
        }
    }

    fn discover(&mut self) -> BackendFuture<'_, MediaSnapshot> {
        self.record("discover");
        if self.stalled == StalledOperation::Discover {
            Box::pin(future::pending())
        } else {
            let current = self.current.clone();
            Box::pin(async move { Ok(current) })
        }
    }

    fn execute<'a>(
        &'a mut self,
        _current: &'a MediaSnapshot,
        _command: &'a MediaCommand,
    ) -> BackendFuture<'a, ()> {
        self.record("command");
        if self.stalled == StalledOperation::Command {
            Box::pin(future::pending())
        } else {
            Box::pin(async { Ok(()) })
        }
    }
}

struct DropSignal(Option<mpsc::SyncSender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(dropped) = self.0.take() {
            let _ = dropped.send(());
        }
    }
}

struct CadenceBackend {
    starts: mpsc::SyncSender<Instant>,
    discovery_duration: Duration,
}

struct RecoveringConnectionBackend {
    attempts: usize,
}

impl MediaBackend for RecoveringConnectionBackend {
    fn connect(&mut self) -> BackendFuture<'_, ()> {
        self.attempts += 1;
        let attempt = self.attempts;
        Box::pin(async move {
            if attempt == 1 {
                Err("private backend detail".to_owned())
            } else {
                Ok(())
            }
        })
    }

    fn discover(&mut self) -> BackendFuture<'_, MediaSnapshot> {
        Box::pin(async { Ok(MediaSnapshot::default()) })
    }

    fn execute<'a>(
        &'a mut self,
        _current: &'a MediaSnapshot,
        _command: &'a MediaCommand,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

impl MediaBackend for CadenceBackend {
    fn connect(&mut self) -> BackendFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn discover(&mut self) -> BackendFuture<'_, MediaSnapshot> {
        self.starts.send(Instant::now()).unwrap();
        let discovery_duration = self.discovery_duration;
        Box::pin(async move {
            tokio::time::sleep(discovery_duration).await;
            Ok(MediaSnapshot::default())
        })
    }

    fn execute<'a>(
        &'a mut self,
        _current: &'a MediaSnapshot,
        _command: &'a MediaCommand,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

fn stalling_backend(
    stalled: StalledOperation,
) -> (
    StallingBackend,
    mpsc::Receiver<BackendEvent>,
    mpsc::Receiver<()>,
) {
    let (events, event_receiver) = mpsc::sync_channel(8);
    let (dropped, dropped_receiver) = mpsc::sync_channel(1);
    let mut current = snapshot("org.mpris.MediaPlayer2.fake", MediaPlaybackStatus::Playing);
    current.capabilities.can_go_next = true;
    (
        StallingBackend {
            stalled,
            events,
            dropped: Some(dropped),
            current,
        },
        event_receiver,
        dropped_receiver,
    )
}

fn receive_event(events: &mpsc::Receiver<BackendEvent>, operation: &'static str) -> BackendEvent {
    loop {
        let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
        if event.operation == operation {
            return event;
        }
    }
}

#[test]
fn media_diagnostic_omits_private_failure_and_reports_recovery() {
    let temp = tempfile::tempdir().expect("create log directory");
    let log_runtime =
        LoggerRuntime::start_in(Component::Overlay, temp.path()).expect("start test logger");
    let client = MediaClient::spawn_with_backend_and_logger(
        RecoveringConnectionBackend { attempts: 0 },
        log_runtime.logger(),
        || {},
        WorkerTiming::for_tests(Duration::from_millis(20), Duration::from_millis(100)),
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if client
            .take_latest()
            .is_some_and(|snapshot| snapshot.value.error.is_none())
        {
            break;
        }
        assert!(Instant::now() < deadline, "media provider did not recover");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(client);
    drop(log_runtime);

    let contents =
        std::fs::read_to_string(temp.path().join("overlay.log")).expect("read diagnostic log");
    assert_eq!(contents.matches("widget_provider_failed").count(), 1);
    assert!(contents.contains("widget=media provider=mpris category=connection"));
    assert!(contents.contains("widget_provider_recovered widget=media provider=mpris"));
    assert!(!contents.contains("private backend detail"));
}

#[test]
fn playing_player_wins_independent_of_discovery_order() {
    let paused = snapshot("org.mpris.MediaPlayer2.z", MediaPlaybackStatus::Paused);
    let playing = snapshot("org.mpris.MediaPlayer2.a", MediaPlaybackStatus::Playing);

    assert_eq!(
        select_player(&[paused.clone(), playing.clone()])
            .and_then(|selected| selected.bus_name.as_deref()),
        Some("org.mpris.MediaPlayer2.a")
    );
    assert_eq!(
        select_player(&[playing, paused]).and_then(|selected| selected.bus_name.as_deref()),
        Some("org.mpris.MediaPlayer2.a")
    );
}

#[test]
fn equal_status_players_use_the_lexically_first_exact_bus_name() {
    let z = snapshot(
        "org.mpris.MediaPlayer2.z-instance",
        MediaPlaybackStatus::Playing,
    );
    let a = snapshot(
        "org.mpris.MediaPlayer2.a-instance",
        MediaPlaybackStatus::Playing,
    );

    assert_eq!(
        select_player(&[z, a]).and_then(|selected| selected.bus_name.as_deref()),
        Some("org.mpris.MediaPlayer2.a-instance")
    );
}

#[test]
fn title_bounds_preserve_utf8_character_boundaries() {
    let input = format!("{}é", "a".repeat(TITLE_MAX_BYTES - 1));
    let bounded = bound_text(&input, TITLE_MAX_BYTES).unwrap();

    assert!(bounded.len() <= TITLE_MAX_BYTES);
    assert!(input.starts_with(&bounded));
    assert!(bounded.is_char_boundary(bounded.len()));
}

#[test]
fn artist_bounds_preserve_utf8_character_boundaries() {
    let artist = format!("{}é", "a".repeat(TITLE_MAX_BYTES - 1));
    let artists = OwnedValue::try_from(Value::from(vec![Str::from(artist)])).unwrap();
    let parsed = parse_metadata(&HashMap::from([("xesam:artist".to_owned(), artists)]));
    let bounded = parsed.artist.unwrap();

    assert!(bounded.len() <= TITLE_MAX_BYTES);
    assert!(bounded.is_char_boundary(bounded.len()));
}

#[test]
fn worker_uses_one_named_current_thread_runtime() {
    let runtime = build_runtime().unwrap();

    assert_eq!(WORKER_THREAD_NAME, "overcrow-mpris-provider");
    assert_eq!(
        runtime.handle().runtime_flavor(),
        tokio::runtime::RuntimeFlavor::CurrentThread
    );
}

#[test]
fn connection_backoff_is_exponential_and_capped_at_five_seconds() {
    let mut backoff = Backoff::new(INITIAL_BACKOFF, MAXIMUM_BACKOFF);

    assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    assert_eq!(backoff.next_delay(), Duration::from_secs(2));
    assert_eq!(backoff.next_delay(), Duration::from_secs(4));
    assert_eq!(backoff.next_delay(), Duration::from_secs(5));
    assert_eq!(backoff.next_delay(), Duration::from_secs(5));
}

#[test]
fn polling_targets_one_interval_between_starts_despite_discovery_time() {
    let (starts, start_receiver) = mpsc::sync_channel(4);
    let client = MediaClient::spawn_with_backend(
        CadenceBackend {
            starts,
            discovery_duration: Duration::from_millis(150),
        },
        || {},
        WorkerTiming::for_tests(Duration::from_millis(250), Duration::from_secs(1)),
    );

    let first = start_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    let _second = start_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    let third = start_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    drop(client);

    assert!(
        third.duration_since(first) < Duration::from_millis(680),
        "poll starts drifted by discovery duration: {:?}",
        third.duration_since(first)
    );
}

#[test]
fn malformed_or_empty_metadata_fields_are_omitted() {
    let metadata = HashMap::from([
        ("xesam:title".to_owned(), OwnedValue::from(true)),
        (
            "xesam:artist".to_owned(),
            OwnedValue::from(Str::from("not-an-array")),
        ),
    ]);

    let parsed = parse_metadata(&metadata);

    assert_eq!(parsed.title, None);
    assert_eq!(parsed.artist, None);
}

#[test]
fn metadata_parser_retains_only_title_and_artist() {
    let artists =
        OwnedValue::try_from(Value::from(vec![Str::from("First"), Str::from("Second")])).unwrap();
    let metadata = HashMap::from([
        (
            "xesam:title".to_owned(),
            OwnedValue::from(Str::from("Track")),
        ),
        ("xesam:artist".to_owned(), artists),
        (
            "mpris:artUrl".to_owned(),
            OwnedValue::from(Str::from("file:///must-not-be-read")),
        ),
        (
            "xesam:url".to_owned(),
            OwnedValue::from(Str::from("https://must-not-be-read.invalid")),
        ),
    ]);

    let parsed = parse_metadata(&metadata);

    assert_eq!(parsed.title.as_deref(), Some("Track"));
    assert_eq!(parsed.artist.as_deref(), Some("First, Second"));
}

#[test]
fn artist_parser_stops_at_the_fixed_artist_count() {
    let artists = (0..9)
        .map(|index| Str::from(format!("Artist {index}")))
        .collect::<Vec<_>>();
    let artists = OwnedValue::try_from(Value::from(artists)).unwrap();
    let parsed = parse_metadata(&HashMap::from([("xesam:artist".to_owned(), artists)]));

    assert_eq!(
        parsed.artist.as_deref(),
        Some("Artist 0, Artist 1, Artist 2, Artist 3, Artist 4, Artist 5, Artist 6, Artist 7")
    );
}

#[test]
fn discovery_accepts_only_current_mpris_well_known_names() {
    assert_eq!(
        filter_mpris_bus_names([
            ":1.42",
            "org.mpris.MediaPlayer2",
            "org.mpris.MediaPlayer2.vlc",
            "org.mpris.MediaPlayer20.lookalike",
            "org.mpris.MediaPlayer2.firefox.instance_1",
        ])
        .unwrap(),
        vec![
            "org.mpris.MediaPlayer2.vlc".to_owned(),
            "org.mpris.MediaPlayer2.firefox.instance_1".to_owned(),
        ]
    );
    assert_eq!(PLAYER_PATH, "/org/mpris/MediaPlayer2");
    assert_eq!(PLAYER_INTERFACE, "org.mpris.MediaPlayer2.Player");
}

#[test]
fn discovery_rejects_more_than_the_fixed_mpris_candidate_limit() {
    let names =
        (0..=MPRIS_CANDIDATE_LIMIT).map(|index| format!("org.mpris.MediaPlayer2.player_{index}"));

    let error = filter_mpris_bus_names(names).unwrap_err();

    assert_eq!(
        error,
        format!("more than {MPRIS_CANDIDATE_LIMIT} MPRIS candidates")
    );
}

#[test]
fn selection_is_updated_incrementally_without_retaining_candidates() {
    let mut selected = None;

    consider_player(
        &mut selected,
        snapshot("org.mpris.MediaPlayer2.z", MediaPlaybackStatus::Paused),
    );
    consider_player(
        &mut selected,
        snapshot("org.mpris.MediaPlayer2.b", MediaPlaybackStatus::Playing),
    );
    consider_player(
        &mut selected,
        snapshot("org.mpris.MediaPlayer2.a", MediaPlaybackStatus::Playing),
    );

    assert_eq!(
        selected.and_then(|player| player.bus_name),
        Some("org.mpris.MediaPlayer2.a".to_owned())
    );
}

#[test]
fn incremental_selection_omits_candidates_without_an_exact_bus_name() {
    let mut selected = None;

    consider_player(&mut selected, MediaSnapshot::default());

    assert_eq!(selected, None);
}

#[test]
fn malformed_playback_status_is_omitted() {
    assert_eq!(
        parse_playback_status("Playing"),
        Some(MediaPlaybackStatus::Playing)
    );
    assert_eq!(parse_playback_status("Buffering"), None);
}

#[test]
fn commands_are_capability_gated_and_keep_the_exact_player_name() {
    let mut current = snapshot(
        "org.mpris.MediaPlayer2.firefox.instance_7",
        MediaPlaybackStatus::Paused,
    );
    current.capabilities.can_play = true;
    current.capabilities.can_go_next = true;

    assert_eq!(
        MediaAction::Previous.command_for(&current),
        None,
        "unsupported controls must never enter the command queue"
    );

    let play = MediaAction::PlayPause.command_for(&current).unwrap();
    assert_eq!(play.bus_name(), "org.mpris.MediaPlayer2.firefox.instance_7");
    assert_eq!(command_method(&current, &play), Some("PlayPause"));

    let next = MediaAction::Next.command_for(&current).unwrap();
    assert_eq!(command_method(&current, &next), Some("Next"));
}

#[test]
fn queued_command_is_discarded_after_selected_player_changes() {
    let mut prior = snapshot("org.mpris.MediaPlayer2.prior", MediaPlaybackStatus::Playing);
    prior.capabilities.can_pause = true;
    let command = MediaAction::PlayPause.command_for(&prior).unwrap();

    let mut current = snapshot(
        "org.mpris.MediaPlayer2.current",
        MediaPlaybackStatus::Playing,
    );
    current.capabilities.can_pause = true;

    assert_eq!(command_method(&current, &command), None);
}

#[test]
fn latest_snapshot_publication_coalesces_pending_values() {
    let (publisher, receiver) = snapshot_channel();
    let first = snapshot("org.mpris.MediaPlayer2.first", MediaPlaybackStatus::Paused);
    let latest = snapshot(
        "org.mpris.MediaPlayer2.latest",
        MediaPlaybackStatus::Playing,
    );

    assert!(publisher.publish(first));
    assert!(publisher.publish(latest.clone()));

    let received = receiver.take_latest().expect("latest snapshot is ready");
    assert_eq!(received.revision, 2);
    assert_eq!(received.value.as_ref(), &latest);
    assert!(receiver.take_latest().is_none());
}

#[test]
fn unchanged_media_frame_does_not_publish_or_replace_the_snapshot() {
    let (publisher, receiver) = snapshot_channel();
    let repaints = std::sync::atomic::AtomicUsize::new(0);
    let current = snapshot(
        "org.mpris.MediaPlayer2.current",
        MediaPlaybackStatus::Playing,
    );

    assert!(publish_if_changed(&publisher, &current, &|| {
        repaints.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
    let first = receiver.take_latest().expect("changed frame is ready");

    assert!(!publish_if_changed(&publisher, &current, &|| {
        repaints.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
    assert!(receiver.take_latest().is_none());
    assert!(std::sync::Arc::ptr_eq(
        &first.value,
        &publisher.current().value
    ));
    assert_eq!(repaints.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn spawn_failure_publishes_and_notifies_only_when_the_error_snapshot_changes() {
    let (publisher, receiver) = snapshot_channel();
    let callbacks = std::sync::atomic::AtomicUsize::new(0);
    let error = std::io::Error::other("worker spawn unavailable");

    assert!(publish_spawn_failure(&publisher, &error, &|| {
        callbacks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
    let published = receiver.take_latest().expect("failure snapshot is ready");
    assert_eq!(published.revision, 1);
    assert_eq!(
        published.value.error.as_deref(),
        Some("worker spawn unavailable")
    );

    assert!(!publish_spawn_failure(&publisher, &error, &|| {
        callbacks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
    assert!(receiver.take_latest().is_none());
    assert_eq!(callbacks.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn media_command_queue_has_a_fixed_non_blocking_bound() {
    let (_publisher, snapshots) = snapshot_channel();
    let (commands, mut command_receiver) = command_channel();
    let (shutdown, _shutdown_receiver) = tokio::sync::watch::channel(false);
    let client = MediaClient::from_parts(snapshots, commands, shutdown, None);
    let mut current = snapshot(
        "org.mpris.MediaPlayer2.player",
        MediaPlaybackStatus::Playing,
    );
    current.capabilities.can_go_next = true;

    for _ in 0..COMMAND_CAPACITY {
        assert!(client.send(&current, MediaAction::Next));
    }
    assert!(!client.send(&current, MediaAction::Next));

    for _ in 0..COMMAND_CAPACITY {
        assert!(command_receiver.try_recv().is_ok());
    }
    assert!(command_receiver.try_recv().is_err());
}

#[test]
fn drop_cancels_stalled_discovery_and_joins_the_named_worker() {
    let (backend, events, backend_dropped) = stalling_backend(StalledOperation::Discover);
    let (repaint_dropped, repaint_dropped_receiver) = mpsc::sync_channel(1);
    let repaint_guard = DropSignal(Some(repaint_dropped));
    let client = MediaClient::spawn_with_backend(
        backend,
        move || {
            let _ = &repaint_guard;
        },
        WorkerTiming::for_tests(Duration::from_millis(20), Duration::from_secs(1)),
    );
    let event = receive_event(&events, "discover");
    assert_eq!(event.thread_name.as_deref(), Some(WORKER_THREAD_NAME));
    let started = Instant::now();
    drop(client);

    assert!(started.elapsed() < Duration::from_millis(500));
    backend_dropped
        .recv_timeout(Duration::from_millis(50))
        .unwrap();
    repaint_dropped_receiver
        .recv_timeout(Duration::from_millis(50))
        .unwrap();
}

#[test]
fn drop_cancels_stalled_connection_and_joins_the_named_worker() {
    let (backend, events, backend_dropped) = stalling_backend(StalledOperation::Connect);
    let (repaint_dropped, repaint_dropped_receiver) = mpsc::sync_channel(1);
    let repaint_guard = DropSignal(Some(repaint_dropped));
    let client = MediaClient::spawn_with_backend(
        backend,
        move || {
            let _ = &repaint_guard;
        },
        WorkerTiming::for_tests(Duration::from_millis(20), Duration::from_secs(1)),
    );
    let event = receive_event(&events, "connect");
    assert_eq!(event.thread_name.as_deref(), Some(WORKER_THREAD_NAME));
    let started = Instant::now();
    drop(client);

    assert!(started.elapsed() < Duration::from_millis(500));
    backend_dropped
        .recv_timeout(Duration::from_millis(50))
        .unwrap();
    repaint_dropped_receiver
        .recv_timeout(Duration::from_millis(50))
        .unwrap();
}

#[test]
fn drop_cancels_stalled_command_and_joins_the_named_worker() {
    let (backend, events, backend_dropped) = stalling_backend(StalledOperation::Command);
    let current = backend.current.clone();
    let (repaint_dropped, repaint_dropped_receiver) = mpsc::sync_channel(1);
    let repaint_guard = DropSignal(Some(repaint_dropped));
    let client = MediaClient::spawn_with_backend(
        backend,
        move || {
            let _ = &repaint_guard;
        },
        WorkerTiming::for_tests(Duration::from_millis(200), Duration::from_secs(1)),
    );
    receive_event(&events, "discover");
    assert!(client.send(&current, MediaAction::Next));
    let event = receive_event(&events, "command");
    assert_eq!(event.thread_name.as_deref(), Some(WORKER_THREAD_NAME));
    let started = Instant::now();
    drop(client);

    assert!(started.elapsed() < Duration::from_millis(500));
    backend_dropped
        .recv_timeout(Duration::from_millis(50))
        .unwrap();
    repaint_dropped_receiver
        .recv_timeout(Duration::from_millis(50))
        .unwrap();
}

#[test]
fn stalled_discovery_has_an_explicit_operation_timeout() {
    let (backend, events, _backend_dropped) = stalling_backend(StalledOperation::Discover);
    let temp = tempfile::tempdir().expect("create log directory");
    let log_runtime =
        LoggerRuntime::start_in(Component::Overlay, temp.path()).expect("start test logger");
    let client = MediaClient::spawn_with_backend_and_logger(
        backend,
        log_runtime.logger(),
        || {},
        WorkerTiming::for_tests(Duration::from_millis(20), Duration::from_millis(30)),
    );
    receive_event(&events, "discover");

    let deadline = Instant::now() + Duration::from_millis(300);
    let error = loop {
        if let Some(error) = client
            .take_latest()
            .and_then(|snapshot| snapshot.value.error.clone())
        {
            break error;
        }
        assert!(
            Instant::now() < deadline,
            "operation timeout was not published"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    drop(client);
    drop(log_runtime);

    assert!(error.contains("MPRIS discovery timed out"));
    let contents =
        std::fs::read_to_string(temp.path().join("overlay.log")).expect("read diagnostic log");
    assert!(contents.contains("widget=media provider=mpris category=timeout"));
}

#[test]
fn provider_errors_are_utf8_bounded() {
    let error = format!("{}é", "x".repeat(MEDIA_ERROR_MAX_BYTES));
    let snapshot = MediaSnapshot::provider_error(&error);
    let retained = snapshot.error.unwrap();

    assert!(retained.len() <= MEDIA_ERROR_MAX_BYTES);
    assert!(retained.is_char_boundary(retained.len()));
    assert_eq!(snapshot.bus_name, None);
}

#[test]
fn missing_player_and_passive_mode_are_read_only() {
    let missing = MediaPresentation::new(&MediaSnapshot::default(), OverlayMode::Interactive);
    assert_eq!(missing.empty_message, Some("No active media"));
    assert!(missing.controls.is_empty());

    let mut playing = snapshot(
        "org.mpris.MediaPlayer2.player",
        MediaPlaybackStatus::Playing,
    );
    playing.capabilities = MediaCapabilities {
        can_go_previous: true,
        can_play: true,
        can_pause: true,
        can_go_next: true,
    };
    let passive = MediaPresentation::new(&playing, OverlayMode::Passive);
    assert!(passive.controls.is_empty());
}

#[test]
fn interactive_presentation_exposes_only_supported_controls() {
    let mut paused = snapshot("org.mpris.MediaPlayer2.player", MediaPlaybackStatus::Paused);
    paused.capabilities.can_play = true;
    paused.capabilities.can_go_next = true;

    let presentation = MediaPresentation::new(&paused, OverlayMode::Interactive);

    assert_eq!(presentation.title, "Song");
    assert_eq!(presentation.artist.as_deref(), Some("Artist"));
    assert_eq!(
        presentation.controls,
        vec![
            MediaControl {
                label: "Play",
                action: MediaAction::PlayPause,
            },
            MediaControl {
                label: "Next",
                action: MediaAction::Next,
            },
        ]
    );
}
