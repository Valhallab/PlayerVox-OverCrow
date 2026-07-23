use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    future::Future,
    io::{self, BufRead, BufReader},
    path::PathBuf,
    pin::Pin,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use std::os::unix::process::ExitStatusExt;

use overcrow_config::{LifecycleSettings, ShortcutSettings, WidgetProfile};
use overcrow_protocol::{Core1Proxy, CoreSnapshot, CoreState, GameWindow, OverlayMode, Rect};
use tokio::sync::{RwLock, mpsc, oneshot, watch};
use zbus::Address;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str};

use crate::shortcut::{
    PortalResponse, RequestEvent, RequestEventSource, RequestPathStrategy, ResponseSource,
    ShortcutAction, ShortcutDefinition, await_monitored_response, desired_shortcuts,
    host_registry_is_unavailable, parse_bind_response, parse_bind_results, parse_create_response,
    portal_owner_change_event, random_portal_token, register_host_portal_identity, request_path,
    request_path_strategy, session_path,
};
use crate::{
    BRIDGE_LEASE_TIMEOUT, CoreRuntime, CoreService, PortalShortcutBroker, ProcessInfo,
    ShortcutAvailability, ShortcutError, ShortcutEvent, ShortcutFuture, ShortcutPolicy,
    ShortcutPortal, ShortcutSession, WindowObservation, XdgPortal, portal_trigger,
};

fn shortcut(enabled: bool, accelerator: &str) -> ShortcutSettings {
    ShortcutSettings {
        enabled,
        accelerator: accelerator.to_owned(),
    }
}

fn active_snapshot(mode: OverlayMode) -> CoreSnapshot {
    CoreSnapshot {
        active_game: Some(GameWindow {
            pid: Some(42),
            steam_app_id: Some(620),
            app_id: Some("portal2".to_owned()),
            title: "Portal 2".to_owned(),
            rect: Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale: 1.0,
            backend: "wayland".to_owned(),
        }),
        overlay_mode: mode,
        session_elapsed_ms: None,
        ..CoreSnapshot::default()
    }
}

#[test]
fn shortcut_policy_requires_an_active_game_and_enabled_setting() {
    let enabled = shortcut(true, "Meta+Alt+O");

    assert!(!ShortcutPolicy::should_bind(
        &CoreSnapshot::default(),
        &enabled
    ));
    assert!(ShortcutPolicy::should_bind(
        &active_snapshot(OverlayMode::Passive),
        &enabled
    ));
    assert!(ShortcutPolicy::should_bind(
        &active_snapshot(OverlayMode::Interactive),
        &enabled
    ));
    assert!(!ShortcutPolicy::should_bind(
        &active_snapshot(OverlayMode::Passive),
        &shortcut(false, "Meta+Alt+O")
    ));
}

#[test]
fn only_exact_missing_registry_errors_allow_identity_fallback() {
    assert!(host_registry_is_unavailable(Some(
        "org.freedesktop.DBus.Error.UnknownInterface"
    )));
    assert!(host_registry_is_unavailable(Some(
        "org.freedesktop.DBus.Error.UnknownMethod"
    )));
    assert!(!host_registry_is_unavailable(Some(
        "org.freedesktop.DBus.Error.AccessDenied"
    )));
    assert!(!host_registry_is_unavailable(Some("UnknownInterface")));
    assert!(!host_registry_is_unavailable(None));
}

#[test]
fn diagnostic_rendering_is_bounded_even_for_a_direct_public_variant() {
    let availability =
        ShortcutAvailability::Unavailable("é".repeat(ShortcutAvailability::MAX_MESSAGE_BYTES));

    let diagnostic = availability.diagnostic();

    assert!(diagnostic.len() <= ShortcutAvailability::MAX_DIAGNOSTIC_BYTES);
    assert!(diagnostic.is_char_boundary(diagnostic.len()));
}

#[test]
fn portal_trigger_normalizes_supported_accelerators_deterministically() {
    assert_eq!(portal_trigger("Meta+Alt+O").unwrap(), "LOGO+ALT+o");
    assert_eq!(
        portal_trigger("Meta+Ctrl+Alt+Shift+9").unwrap(),
        "LOGO+CTRL+ALT+SHIFT+9"
    );
}

#[test]
fn portal_trigger_rejects_unsupported_or_ambiguous_grammar() {
    for accelerator in [
        "",
        "O",
        "Super+O",
        "meta+O",
        "Meta+Alt+Alt+O",
        "Alt+Meta+O",
        "Meta+F1",
        "Meta+?",
        "Meta+é",
        "Meta++O",
    ] {
        assert!(
            portal_trigger(accelerator).is_err(),
            "accelerator {accelerator:?} must be rejected"
        );
    }
}

#[tokio::test]
async fn settings_only_reload_wakes_shortcut_reconciliation() {
    let initial = LifecycleSettings {
        enabled: true,
        shortcut: shortcut(true, "Meta+Alt+O"),
        ..LifecycleSettings::default()
    };
    let runtime = CoreRuntime::with_settings(
        Arc::new(RwLock::new(CoreState::default())),
        HashMap::new(),
        initial.clone(),
    )
    .await;
    let mut shortcut_settings = runtime.shortcut_settings();
    let initial_snapshot = runtime.snapshot().await;
    let mut changed = initial;
    changed.shortcut.accelerator = "Meta+Alt+P".to_owned();

    let resulting_snapshot = runtime.reload_settings(changed).await.unwrap();
    tokio::time::timeout(Duration::from_millis(100), shortcut_settings.changed())
        .await
        .expect("settings-only reload must wake the watch")
        .expect("shortcut settings watch remains open");

    assert_eq!(resulting_snapshot, initial_snapshot);
    assert_eq!(shortcut_settings.borrow().accelerator, "Meta+Alt+P");
}

#[derive(Clone)]
struct FakePortal {
    attempts: mpsc::UnboundedSender<BindAttempt>,
    state: Arc<FakePortalState>,
}

#[derive(Default)]
struct FakePortalState {
    events: Mutex<Vec<String>>,
    in_flight: AtomicUsize,
    live_sessions: AtomicUsize,
    closes: AtomicUsize,
}

struct BindAttempt {
    definitions: Vec<ShortcutDefinition>,
    response: oneshot::Sender<Result<Box<dyn ShortcutSession>, ShortcutError>>,
    state: Arc<FakePortalState>,
}

impl BindAttempt {
    fn respond_session(self, handle: &str) -> mpsc::UnboundedSender<ShortcutEvent> {
        self.respond_session_with_close(handle, CloseBehavior::Success)
    }

    fn respond_session_with_close(
        self,
        handle: &str,
        close_behavior: CloseBehavior,
    ) -> mpsc::UnboundedSender<ShortcutEvent> {
        let (events, event_rx) = mpsc::unbounded_channel();
        self.state.live_sessions.fetch_add(1, Ordering::SeqCst);
        let session = FakeSession {
            handle: handle.to_owned(),
            events: event_rx,
            state: Arc::clone(&self.state),
            close_behavior,
            released: false,
        };
        assert!(self.response.send(Ok(Box::new(session))).is_ok());
        events
    }

    fn deny(self, message: &str) {
        assert!(self.response.send(Err(ShortcutError::new(message))).is_ok());
    }
}

struct InFlightGuard(Arc<FakePortalState>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ShortcutPortal for FakePortal {
    fn bind(
        &self,
        definitions: Vec<ShortcutDefinition>,
    ) -> ShortcutFuture<'static, Result<Box<dyn ShortcutSession>, ShortcutError>> {
        let (response, response_rx) = oneshot::channel();
        self.state.in_flight.fetch_add(1, Ordering::SeqCst);
        self.state.events.lock().unwrap().push(format!(
            "bind:{}",
            definitions
                .iter()
                .map(|definition| definition.accelerator.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));
        self.attempts
            .send(BindAttempt {
                definitions,
                response,
                state: Arc::clone(&self.state),
            })
            .unwrap();
        let guard = InFlightGuard(Arc::clone(&self.state));
        Box::pin(async move {
            let _guard = guard;
            response_rx
                .await
                .map_err(|_| ShortcutError::new("fake bind response dropped"))?
        })
    }
}

struct FakeSession {
    handle: String,
    events: mpsc::UnboundedReceiver<ShortcutEvent>,
    state: Arc<FakePortalState>,
    close_behavior: CloseBehavior,
    released: bool,
}

#[derive(Clone, Copy)]
enum CloseBehavior {
    Success,
    Failure,
    Pending,
}

impl FakeSession {
    fn release(&mut self) {
        if !self.released {
            self.released = true;
            self.state.live_sessions.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for FakeSession {
    fn drop(&mut self) {
        self.release();
    }
}

impl ShortcutSession for FakeSession {
    fn handle(&self) -> &str {
        &self.handle
    }

    fn next_event(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<ShortcutEvent, ShortcutError>> + Send + '_>> {
        Box::pin(async {
            self.events
                .recv()
                .await
                .ok_or_else(|| ShortcutError::new("fake activation stream closed"))
        })
    }

    fn close(mut self: Box<Self>) -> ShortcutFuture<'static, Result<(), ShortcutError>> {
        self.state.closes.fetch_add(1, Ordering::SeqCst);
        self.state.events.lock().unwrap().push("close".to_owned());
        match self.close_behavior {
            CloseBehavior::Success => {
                self.release();
                Box::pin(async move { Ok(()) })
            }
            CloseBehavior::Failure => {
                self.release();
                Box::pin(async move { Err(ShortcutError::new("fake close failed")) })
            }
            CloseBehavior::Pending => Box::pin(async move {
                std::future::pending::<()>().await;
                unreachable!()
            }),
        }
    }
}

fn fake_portal() -> (
    FakePortal,
    mpsc::UnboundedReceiver<BindAttempt>,
    Arc<FakePortalState>,
) {
    let (attempts, attempt_rx) = mpsc::unbounded_channel();
    let state = Arc::new(FakePortalState::default());
    (
        FakePortal {
            attempts,
            state: Arc::clone(&state),
        },
        attempt_rx,
        state,
    )
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

fn sample_observation() -> WindowObservation {
    WindowObservation {
        pid: Some(42),
        app_id: Some("portal2".to_owned()),
        title: "Portal 2".to_owned(),
        rect: Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
        scale: 1.0,
        backend: "wayland".to_owned(),
    }
}

fn enabled_settings(accelerator: &str) -> LifecycleSettings {
    LifecycleSettings {
        enabled: true,
        selected_steam_app_ids: BTreeSet::from([620]),
        shortcut: shortcut(true, accelerator),
        ..LifecycleSettings::default()
    }
}

async fn active_runtime() -> CoreRuntime {
    let runtime = CoreRuntime::with_settings(
        Arc::new(RwLock::new(CoreState::default())),
        HashMap::from([(42, sample_process())]),
        enabled_settings("Meta+Alt+O"),
    )
    .await;
    runtime.apply_bridge_observation(sample_observation()).await;
    runtime
}

fn enabled_profile() -> WidgetProfile {
    let mut profile = WidgetProfile::default();
    profile.manual_stopwatch.enabled = true;
    profile
}

async fn active_runtime_with_profile(profile: WidgetProfile) -> CoreRuntime {
    let runtime = CoreRuntime::with_settings_and_widget_profile(
        Arc::new(RwLock::new(CoreState::default())),
        HashMap::from([(42, sample_process())]),
        enabled_settings("Meta+Alt+O"),
        profile,
    )
    .await;
    runtime.apply_bridge_observation(sample_observation()).await;
    runtime
}

#[test]
fn desired_shortcuts_are_fixed_ordered_and_game_scoped() {
    let settings = shortcut(true, "Meta+Alt+O");
    let active = active_snapshot(OverlayMode::Passive);

    assert_eq!(
        desired_shortcuts(&active, &settings, &enabled_profile())
            .unwrap()
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![
            "toggle-overlay",
            "toggle-manual-stopwatch",
            "reset-manual-stopwatch",
        ],
    );
    assert_eq!(
        desired_shortcuts(&active, &settings, &WidgetProfile::default())
            .unwrap()
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec!["toggle-overlay"],
    );
    assert!(
        desired_shortcuts(&CoreSnapshot::default(), &settings, &enabled_profile())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn manual_shortcuts_do_not_depend_on_the_overlay_shortcut_setting() {
    let definitions = desired_shortcuts(
        &active_snapshot(OverlayMode::Passive),
        &shortcut(false, "Meta+Alt+O"),
        &enabled_profile(),
    )
    .unwrap();

    assert_eq!(
        definitions
            .iter()
            .map(|item| (item.id, item.accelerator.as_str(), item.action))
            .collect::<Vec<_>>(),
        vec![
            (
                "toggle-manual-stopwatch",
                "Meta+Alt+P",
                ShortcutAction::ToggleManualStopwatch,
            ),
            (
                "reset-manual-stopwatch",
                "Meta+Alt+R",
                ShortcutAction::ResetManualStopwatch,
            ),
        ],
    );
}

#[test]
fn desired_shortcuts_reject_duplicate_canonical_accelerators() {
    // Overlay toggle must not share an accelerator with the stopwatch (Meta+Alt+P).
    assert!(
        desired_shortcuts(
            &active_snapshot(OverlayMode::Passive),
            &shortcut(true, "Meta+Alt+P"),
            &enabled_profile(),
        )
        .is_err()
    );
}

async fn wait_for_availability(
    availability: &mut watch::Receiver<ShortcutAvailability>,
    expected: impl Fn(&ShortcutAvailability) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if expected(&availability.borrow()) {
                break;
            }
            availability.changed().await.unwrap();
        }
    })
    .await
    .expect("shortcut availability transition timed out");
}

#[tokio::test]
async fn broker_coalesces_identical_policy_and_closes_before_rebinding() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));

    let first = attempts.recv().await.unwrap();
    assert_eq!(
        first
            .definitions
            .iter()
            .map(|definition| (definition.id, definition.accelerator.as_str()))
            .collect::<Vec<_>>(),
        [("toggle-overlay", "Meta+Alt+O")]
    );
    let _first_events = first.respond_session("/session/first");
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;

    runtime.set_overlay_interactive(true).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), attempts.recv())
            .await
            .is_err()
    );

    runtime
        .reload_settings(enabled_settings("Meta+Alt+P"))
        .await
        .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(1), attempts.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.definitions[0].accelerator, "Meta+Alt+P");
    assert_eq!(
        state.events.lock().unwrap().as_slice(),
        ["bind:Meta+Alt+O", "close", "bind:Meta+Alt+P"]
    );
    let _second_events = second.respond_session("/session/second");
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
    assert_eq!(state.closes.load(Ordering::SeqCst), 2);
    assert_eq!(state.live_sessions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn exact_activation_toggles_once_and_unrelated_signals_are_ignored() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, _state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let task = tokio::spawn(broker.run(shutdown_rx));
    let events = attempts
        .recv()
        .await
        .unwrap()
        .respond_session("/session/expected");

    events
        .send(ShortcutEvent::Activated {
            session_handle: "/session/wrong".to_owned(),
            shortcut_id: "toggle-overlay".to_owned(),
        })
        .unwrap();
    events
        .send(ShortcutEvent::Activated {
            session_handle: "/session/expected".to_owned(),
            shortcut_id: "wrong-id".to_owned(),
        })
        .unwrap();
    events.send(ShortcutEvent::Malformed).unwrap();
    tokio::task::yield_now().await;
    assert_eq!(runtime.snapshot().await.overlay_mode, OverlayMode::Passive);

    events
        .send(ShortcutEvent::Activated {
            session_handle: "/session/expected".to_owned(),
            shortcut_id: "toggle-overlay".to_owned(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.snapshot().await.overlay_mode != OverlayMode::Interactive {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn one_owned_session_dispatches_each_known_action_and_ignores_unknown_ids() {
    let runtime = active_runtime_with_profile(enabled_profile()).await;
    let (portal, mut attempts, _state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let task = tokio::spawn(broker.run(shutdown_rx));
    let attempt = attempts.recv().await.unwrap();
    assert_eq!(
        attempt
            .definitions
            .iter()
            .map(|definition| definition.id)
            .collect::<Vec<_>>(),
        [
            "toggle-overlay",
            "toggle-manual-stopwatch",
            "reset-manual-stopwatch",
        ]
    );
    let events = attempt.respond_session("/session/multi");

    events
        .send(ShortcutEvent::Activated {
            session_handle: "/session/multi".to_owned(),
            shortcut_id: "toggle-manual-stopwatch".to_owned(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !runtime.snapshot().await.manual_stopwatch.running {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    events
        .send(ShortcutEvent::Activated {
            session_handle: "/session/multi".to_owned(),
            shortcut_id: "unknown-action".to_owned(),
        })
        .unwrap();
    tokio::task::yield_now().await;
    assert!(runtime.snapshot().await.manual_stopwatch.running);
    assert_eq!(runtime.snapshot().await.overlay_mode, OverlayMode::Passive);

    events
        .send(ShortcutEvent::Activated {
            session_handle: "/session/multi".to_owned(),
            shortcut_id: "toggle-overlay".to_owned(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.snapshot().await.overlay_mode != OverlayMode::Interactive {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    events
        .send(ShortcutEvent::Activated {
            session_handle: "/session/multi".to_owned(),
            shortcut_id: "reset-manual-stopwatch".to_owned(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.snapshot().await.manual_stopwatch.running {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(runtime.snapshot().await.manual_stopwatch.elapsed_ms, 0);

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn disabling_the_manual_widget_rebinds_without_manual_actions() {
    let runtime = active_runtime_with_profile(enabled_profile()).await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));
    let first = attempts.recv().await.unwrap();
    let _events = first.respond_session("/session/manual-enabled");
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;

    runtime
        .reload_widget_profile(WidgetProfile::default())
        .await
        .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(1), attempts.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second
            .definitions
            .iter()
            .map(|definition| definition.id)
            .collect::<Vec<_>>(),
        ["toggle-overlay"]
    );
    assert_eq!(state.closes.load(Ordering::SeqCst), 1);
    let _events = second.respond_session("/session/manual-disabled");

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn policy_release_cancels_an_in_flight_bind_without_a_live_session() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let task = tokio::spawn(broker.run(shutdown_rx));

    let pending = attempts.recv().await.unwrap();
    assert_eq!(state.in_flight.load(Ordering::SeqCst), 1);
    runtime.clear_game().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while state.in_flight.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        pending
            .response
            .send(Err(ShortcutError::new("late")))
            .is_err()
    );
    assert_eq!(state.live_sessions.load(Ordering::SeqCst), 0);

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn denial_is_non_fatal_bounded_and_not_retried_without_policy_change() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, _state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));

    attempts
        .recv()
        .await
        .unwrap()
        .deny(&"é".repeat(ShortcutAvailability::MAX_MESSAGE_BYTES));
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Unavailable(_))
    })
    .await;
    let ShortcutAvailability::Unavailable(message) = availability.borrow().clone() else {
        unreachable!();
    };
    assert!(message.len() <= ShortcutAvailability::MAX_MESSAGE_BYTES);

    runtime.set_overlay_interactive(true).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), attempts.recv())
            .await
            .is_err()
    );
    assert!(!task.is_finished());

    runtime
        .reload_settings(enabled_settings("Meta+Alt+P"))
        .await
        .unwrap();
    let changed = tokio::time::timeout(Duration::from_secs(1), attempts.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.definitions[0].accelerator, "Meta+Alt+P");
    let _events = changed.respond_session("/session/retry-after-change");
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn activation_stream_loss_is_non_fatal_and_closes_once() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime, portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));
    let events = attempts
        .recv()
        .await
        .unwrap()
        .respond_session("/session/closed");

    drop(events);
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Unavailable(_))
    })
    .await;
    assert_eq!(state.closes.load(Ordering::SeqCst), 1);
    assert!(!task.is_finished());

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
    assert_eq!(state.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shortcut_disable_and_active_game_clear_close_live_sessions() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));
    let _events = attempts
        .recv()
        .await
        .unwrap()
        .respond_session("/session/disable");
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;

    let mut disabled = enabled_settings("Meta+Alt+O");
    disabled.shortcut.enabled = false;
    runtime.reload_settings(disabled).await.unwrap();
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Disabled)
    })
    .await;
    assert_eq!(state.closes.load(Ordering::SeqCst), 1);

    let mut enabled = enabled_settings("Meta+Alt+O");
    enabled.shortcut.enabled = true;
    runtime.reload_settings(enabled).await.unwrap();
    let _events = attempts
        .recv()
        .await
        .unwrap()
        .respond_session("/session/clear");
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;
    runtime.clear_game().await;
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Disabled)
    })
    .await;
    assert_eq!(state.closes.load(Ordering::SeqCst), 2);

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn bridge_lease_expiry_releases_the_live_shortcut() {
    let runtime = active_runtime().await;
    let reported_at = Instant::now();
    runtime
        .apply_bridge_observation_at(sample_observation(), reported_at)
        .await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));
    let _events = attempts
        .recv()
        .await
        .unwrap()
        .respond_session("/session/lease");
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;

    assert!(
        runtime
            .expire_bridge_lease_at(reported_at + BRIDGE_LEASE_TIMEOUT)
            .await
    );
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Disabled)
    })
    .await;
    assert_eq!(state.closes.load(Ordering::SeqCst), 1);

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_cancels_an_in_flight_bind() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime, portal);
    let task = tokio::spawn(broker.run(shutdown_rx));
    let pending = attempts.recv().await.unwrap();

    shutdown_tx.send_replace(true);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert!(
        pending
            .response
            .send(Err(ShortcutError::new("late")))
            .is_err()
    );
    assert_eq!(state.in_flight.load(Ordering::SeqCst), 0);
    assert_eq!(state.live_sessions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn close_failure_is_reported_and_prevents_an_unverified_rebind() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));
    let _events = attempts
        .recv()
        .await
        .unwrap()
        .respond_session_with_close("/session/failing-close", CloseBehavior::Failure);
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;

    runtime
        .reload_settings(enabled_settings("Meta+Alt+P"))
        .await
        .unwrap();
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Unavailable(message) if message.contains("fake close failed"))
    })
    .await;
    assert_eq!(state.closes.load(Ordering::SeqCst), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), attempts.recv())
            .await
            .is_err()
    );

    let mut disabled = enabled_settings("Meta+Alt+P");
    disabled.shortcut.enabled = false;
    runtime.reload_settings(disabled).await.unwrap();
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Disabled)
    })
    .await;
    runtime
        .reload_settings(enabled_settings("Meta+Alt+P"))
        .await
        .unwrap();
    let retry = tokio::time::timeout(Duration::from_secs(1), attempts.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retry.definitions[0].accelerator, "Meta+Alt+P");
    let _events = retry.respond_session("/session/retry");

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_bounds_a_stuck_session_close_and_reports_the_error() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime, portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));
    let _events = attempts
        .recv()
        .await
        .unwrap()
        .respond_session_with_close("/session/stuck-close", CloseBehavior::Pending);
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;

    shutdown_tx.send_replace(true);
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("broker shutdown must remain bounded")
        .unwrap()
        .unwrap_err();

    assert!(error.to_string().contains("timed out"));
    assert_eq!(state.closes.load(Ordering::SeqCst), 1);
    assert_eq!(state.live_sessions.load(Ordering::SeqCst), 0);
}

trait ChildProcess: Send + 'static {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    fn kill(&mut self) -> io::Result<()>;
    fn wait(&mut self) -> io::Result<ExitStatus>;
}

impl ChildProcess for Child {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Child::try_wait(self)
    }

    fn kill(&mut self) -> io::Result<()> {
        Child::kill(self)
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        Child::wait(self)
    }
}

#[derive(Default)]
struct ReapStatus {
    reaped: bool,
    last_error: Option<String>,
}

#[derive(Clone)]
struct ReapTicket {
    status: Arc<(Mutex<ReapStatus>, Condvar)>,
}

impl ReapTicket {
    fn new() -> Self {
        Self {
            status: Arc::new((Mutex::new(ReapStatus::default()), Condvar::new())),
        }
    }

    fn record_error(&self, error: impl Into<String>) {
        let (status, changed) = &*self.status;
        status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .last_error = Some(error.into());
        changed.notify_all();
    }

    fn complete(&self) {
        let (status, changed) = &*self.status;
        status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reaped = true;
        changed.notify_all();
    }

    fn wait_for_reap(&self, timeout: Duration) -> Result<(), String> {
        let (status, changed) = &*self.status;
        let deadline = Instant::now() + timeout;
        let mut status = status.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if status.reaped {
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(status.last_error.clone().map_or_else(
                    || {
                        "timed out waiting for child reap; owner worker retains the child"
                            .to_owned()
                    },
                    |error| {
                        format!(
                            "{error}; timed out waiting for child reap; owner worker retains the child"
                        )
                    },
                ));
            }
            let remaining = deadline - now;
            let waited = changed.wait_timeout(status, remaining);
            let (next, _) = waited.unwrap_or_else(|error| error.into_inner());
            status = next;
        }
    }
}

struct ReapJob<P> {
    process: P,
    ticket: ReapTicket,
    reaped: Arc<AtomicBool>,
}

struct ReaperQueue<P> {
    jobs: VecDeque<ReapJob<P>>,
    closed: bool,
}

struct ReaperState<P> {
    queue: Mutex<ReaperQueue<P>>,
    changed: Condvar,
}

type ReaperWorker = Box<dyn FnOnce() + Send + 'static>;

struct ProcessOwnerReaper<P> {
    state: Arc<ReaperState<P>>,
}

impl<P: ChildProcess> ProcessOwnerReaper<P> {
    fn start_with(spawn_worker: impl FnOnce(ReaperWorker) -> io::Result<()>) -> io::Result<Self> {
        let state = Arc::new(ReaperState {
            queue: Mutex::new(ReaperQueue {
                jobs: VecDeque::new(),
                closed: false,
            }),
            changed: Condvar::new(),
        });
        let worker_state = Arc::clone(&state);
        spawn_worker(Box::new(move || owner_reaper_worker(worker_state)))?;
        Ok(Self { state })
    }

    fn submit(&self, process: P, reaped: Arc<AtomicBool>) -> ReapTicket {
        let ticket = ReapTicket::new();
        let mut queue = self
            .state
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.jobs.push_back(ReapJob {
            process,
            ticket: ticket.clone(),
            reaped,
        });
        drop(queue);
        self.state.changed.notify_one();
        ticket
    }
}

impl<P> Drop for ProcessOwnerReaper<P> {
    fn drop(&mut self) {
        let mut queue = self
            .state
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.closed = true;
        drop(queue);
        self.state.changed.notify_one();
    }
}

fn owner_reaper_worker<P: ChildProcess>(state: Arc<ReaperState<P>>) {
    loop {
        let job = {
            let mut queue = state
                .queue
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while queue.jobs.is_empty() && !queue.closed {
                queue = state
                    .changed
                    .wait(queue)
                    .unwrap_or_else(|error| error.into_inner());
            }
            match queue.jobs.pop_front() {
                Some(job) => job,
                None if queue.closed => return,
                None => continue,
            }
        };
        reap_to_completion(job);
    }
}

fn reap_to_completion<P: ChildProcess>(mut job: ReapJob<P>) {
    match job.process.try_wait() {
        Ok(Some(_)) => {
            job.reaped.store(true, Ordering::SeqCst);
            job.ticket.complete();
            return;
        }
        Ok(None) => {}
        Err(error) => job
            .ticket
            .record_error(format!("initial child status check failed: {error}")),
    }
    if let Err(error) = job.process.kill() {
        job.ticket
            .record_error(format!("failed to kill child: {error}"));
    }

    // Only this dedicated owner thread may block in wait(). It already owns the Child and never
    // releases it after an error; retrying preserves ownership until a real wait succeeds.
    loop {
        match job.process.wait() {
            Ok(_) => {
                job.reaped.store(true, Ordering::SeqCst);
                job.ticket.complete();
                return;
            }
            Err(error) => {
                job.ticket
                    .record_error(format!("failed to reap child: {error}"));
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

struct ReaperOwnedProcess<P: ChildProcess> {
    process: Option<P>,
    ticket: Option<ReapTicket>,
    reaper: ProcessOwnerReaper<P>,
    reaped: Arc<AtomicBool>,
    cleanup_error: Arc<Mutex<Option<String>>>,
}

impl<P: ChildProcess> ReaperOwnedProcess<P> {
    fn spawn(process_factory: impl FnOnce() -> io::Result<P>) -> Result<Self, String> {
        Self::spawn_with(
            |worker| {
                std::thread::Builder::new()
                    .name("overcrow-dbus-reaper".to_owned())
                    .spawn(worker)
                    .map(|_| ())
            },
            process_factory,
        )
    }

    fn spawn_with(
        spawn_worker: impl FnOnce(ReaperWorker) -> io::Result<()>,
        process_factory: impl FnOnce() -> io::Result<P>,
    ) -> Result<Self, String> {
        // The owner thread must exist before the process factory is allowed to create a child.
        let reaper = ProcessOwnerReaper::start_with(spawn_worker)
            .map_err(|error| format!("failed to start child owner reaper: {error}"))?;
        let reaped = Arc::new(AtomicBool::new(false));
        let cleanup_error = Arc::new(Mutex::new(None));
        let process =
            process_factory().map_err(|error| format!("failed to start child: {error}"))?;
        Ok(Self {
            process: Some(process),
            ticket: None,
            reaper,
            reaped,
            cleanup_error,
        })
    }

    fn process_mut(&mut self) -> Option<&mut P> {
        self.process.as_mut()
    }

    fn terminate_and_reap(&mut self, timeout: Duration) -> Result<(), String> {
        if self.ticket.is_none() {
            let process = self
                .process
                .take()
                .ok_or_else(|| "child ownership is unavailable".to_owned())?;
            self.ticket = Some(self.reaper.submit(process, Arc::clone(&self.reaped)));
        }
        let result = self
            .ticket
            .as_ref()
            .expect("reap ticket exists after ownership transfer")
            .wait_for_reap(timeout);
        if let Err(error) = &result {
            *self
                .cleanup_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error.clone());
        }
        result
    }
}

impl<P: ChildProcess> Drop for ReaperOwnedProcess<P> {
    fn drop(&mut self) {
        let _ = self.terminate_and_reap(Duration::from_millis(100));
    }
}

fn complete_process_startup<P: ChildProcess, T>(
    mut process: ReaperOwnedProcess<P>,
    initialize: impl FnOnce(&mut P) -> Result<T, String>,
) -> Result<(ReaperOwnedProcess<P>, T), String> {
    let initialized = initialize(
        process
            .process_mut()
            .ok_or_else(|| "child ownership is unavailable during startup".to_owned())?,
    )?;
    Ok((process, initialized))
}

struct ControlledProcess {
    release: Arc<(Mutex<bool>, Condvar)>,
    wait_started: Arc<AtomicBool>,
    dropped_without_reap: Arc<AtomicBool>,
    wait_errors_remaining: usize,
    reaped: bool,
}

impl ChildProcess for ControlledProcess {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(self.reaped.then(|| ExitStatus::from_raw(0)))
    }

    fn kill(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        if self.wait_errors_remaining > 0 {
            self.wait_errors_remaining -= 1;
            return Err(io::Error::other("injected worker wait failure"));
        }
        self.wait_started.store(true, Ordering::SeqCst);
        let (lock, ready) = &*self.release;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = ready.wait(released).unwrap();
        }
        self.reaped = true;
        Ok(ExitStatus::from_raw(0))
    }
}

impl Drop for ControlledProcess {
    fn drop(&mut self) {
        if !self.reaped {
            self.dropped_without_reap.store(true, Ordering::SeqCst);
        }
    }
}

struct ControlledFixture {
    process: ControlledProcess,
    release: Arc<(Mutex<bool>, Condvar)>,
    wait_started: Arc<AtomicBool>,
    dropped_without_reap: Arc<AtomicBool>,
}

fn controlled_process(wait_errors_remaining: usize) -> ControlledFixture {
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let wait_started = Arc::new(AtomicBool::new(false));
    let dropped_without_reap = Arc::new(AtomicBool::new(false));
    ControlledFixture {
        process: ControlledProcess {
            release: Arc::clone(&release),
            wait_started: Arc::clone(&wait_started),
            dropped_without_reap: Arc::clone(&dropped_without_reap),
            wait_errors_remaining,
            reaped: false,
        },
        release,
        wait_started,
        dropped_without_reap,
    }
}

fn release_controlled_process(release: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, ready) = &**release;
    *lock.lock().unwrap() = true;
    ready.notify_all();
}

fn wait_for_atomic_flag(flag: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !flag.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(flag.load(Ordering::SeqCst), "atomic witness timed out");
}

#[test]
fn reaper_creation_failure_prevents_the_child_factory_from_running() {
    let child_factory_called = Arc::new(AtomicBool::new(false));
    let called = Arc::clone(&child_factory_called);

    let result = ReaperOwnedProcess::<ControlledProcess>::spawn_with(
        |_worker| Err(io::Error::other("injected reaper spawn failure")),
        move || {
            called.store(true, Ordering::SeqCst);
            Ok(controlled_process(0).process)
        },
    );

    assert!(result.is_err());
    assert!(!child_factory_called.load(Ordering::SeqCst));
}

#[test]
fn caller_timeout_leaves_the_worker_owning_the_child_until_late_reap() {
    let ControlledFixture {
        process,
        release,
        wait_started,
        dropped_without_reap,
    } = controlled_process(0);
    let mut owned = ReaperOwnedProcess::spawn(move || Ok(process)).unwrap();
    let reaped = Arc::clone(&owned.reaped);
    let started = Instant::now();

    let error = owned
        .terminate_and_reap(Duration::from_millis(15))
        .unwrap_err();

    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(error.contains("timed out"));
    wait_for_atomic_flag(&wait_started);
    assert!(!reaped.load(Ordering::SeqCst));
    assert!(!dropped_without_reap.load(Ordering::SeqCst));

    release_controlled_process(&release);
    owned
        .terminate_and_reap(Duration::from_secs(1))
        .expect("the existing worker must acknowledge the later real reap");
    assert!(reaped.load(Ordering::SeqCst));
    assert!(!dropped_without_reap.load(Ordering::SeqCst));
}

#[test]
fn startup_failure_transfers_the_child_to_the_preexisting_reaper() {
    let ControlledFixture {
        process,
        release,
        wait_started,
        dropped_without_reap,
    } = controlled_process(0);
    let owned = ReaperOwnedProcess::spawn(move || Ok(process)).unwrap();
    let reaped = Arc::clone(&owned.reaped);
    let started = Instant::now();

    let result: Result<(ReaperOwnedProcess<ControlledProcess>, ()), String> =
        complete_process_startup(owned, |_process| {
            Err("injected startup read failure".to_owned())
        });

    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_millis(250));
    wait_for_atomic_flag(&wait_started);
    assert!(!dropped_without_reap.load(Ordering::SeqCst));
    release_controlled_process(&release);
    wait_for_atomic_flag(&reaped);
    assert!(!dropped_without_reap.load(Ordering::SeqCst));
}

#[test]
fn worker_wait_error_never_sets_a_false_reaped_status() {
    let ControlledFixture {
        process,
        release,
        wait_started,
        dropped_without_reap,
    } = controlled_process(1);
    let mut owned = ReaperOwnedProcess::spawn(move || Ok(process)).unwrap();
    let reaped = Arc::clone(&owned.reaped);

    assert!(owned.terminate_and_reap(Duration::from_millis(15)).is_err());
    wait_for_atomic_flag(&wait_started);
    assert!(!reaped.load(Ordering::SeqCst));
    assert!(!dropped_without_reap.load(Ordering::SeqCst));

    release_controlled_process(&release);
    owned
        .terminate_and_reap(Duration::from_secs(1))
        .expect("a later successful wait must produce the only reap acknowledgement");
    assert!(reaped.load(Ordering::SeqCst));
    assert!(!dropped_without_reap.load(Ordering::SeqCst));
}

type ReapedChild = ReaperOwnedProcess<Child>;

struct IsolatedSessionBus {
    process: ReapedChild,
    address: Address,
    _directory: tempfile::TempDir,
}

impl IsolatedSessionBus {
    async fn start() -> Self {
        let directory = tempfile::tempdir().expect("create isolated D-Bus directory");
        let socket = directory.path().join("bus.sock");
        let config = directory.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                socket.display()
            ),
        )
        .expect("write isolated D-Bus config");

        let process = ReapedChild::spawn(|| {
            Command::new("/usr/bin/dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .args(["--nofork", "--print-address=1", "--nopidfile"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
        })
        .expect("start isolated D-Bus daemon with its owner reaper");
        let (process, stdout) = complete_process_startup(process, |child| {
            child
                .stdout
                .take()
                .ok_or_else(|| "isolated bus stdout was not captured".to_owned())
        })
        .expect("capture isolated bus address");
        let reader = tokio::task::spawn_blocking(move || {
            let mut line = String::new();
            BufReader::new(stdout).read_line(&mut line).map(|_| line)
        });
        let line = tokio::time::timeout(Duration::from_secs(1), reader)
            .await
            .expect("isolated bus address timed out")
            .expect("isolated bus address reader panicked")
            .expect("read isolated bus address");
        assert!(!line.trim().is_empty(), "isolated bus omitted its address");
        let address = line.trim().parse().expect("parse isolated bus address");
        Self {
            process,
            address,
            _directory: directory,
        }
    }

    async fn assert_portal_is_not_activatable(&self) {
        let connection = zbus::connection::Builder::address(self.address.clone())
            .unwrap()
            .build()
            .await
            .expect("connect to isolated bus");
        let dbus = zbus::fdo::DBusProxy::new(&connection)
            .await
            .expect("create isolated bus proxy");
        let activatable = dbus
            .list_activatable_names()
            .await
            .expect("list isolated activatable names");
        assert!(
            activatable
                .iter()
                .all(|name| name.as_str() != "org.freedesktop.portal.Desktop"),
            "dedicated config must not load standard portal service directories"
        );
    }

    async fn shutdown(mut self) {
        let reaped = Arc::clone(&self.process.reaped);
        self.process
            .terminate_and_reap(Duration::from_secs(1))
            .expect("isolated bus reap acknowledgement timed out");
        assert!(
            reaped.load(Ordering::SeqCst),
            "isolated bus was not reaped: {:?}",
            self.process.cleanup_error.lock().unwrap().as_deref()
        );
    }
}

#[derive(Clone)]
struct RecordingHostRegistry {
    app_ids: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.freedesktop.host.portal.Registry")]
impl RecordingHostRegistry {
    fn register(&self, app_id: &str, _options: HashMap<String, OwnedValue>) {
        self.app_ids
            .lock()
            .expect("registry recording lock")
            .push(app_id.to_owned());
    }
}

#[tokio::test]
async fn native_portal_identity_uses_the_installed_desktop_id() {
    let bus = IsolatedSessionBus::start().await;
    let app_ids = Arc::new(Mutex::new(Vec::new()));
    let server = zbus::connection::Builder::address(bus.address.clone())
        .expect("valid isolated bus address")
        .name("org.freedesktop.portal.Desktop")
        .expect("valid portal bus name")
        .serve_at(
            "/org/freedesktop/portal/desktop",
            RecordingHostRegistry {
                app_ids: Arc::clone(&app_ids),
            },
        )
        .expect("serve host registry")
        .build()
        .await
        .expect("start host registry");
    let client = zbus::connection::Builder::address(bus.address.clone())
        .expect("valid isolated bus address")
        .build()
        .await
        .expect("connect portal client");

    register_host_portal_identity(&client)
        .await
        .expect("register host portal identity");

    assert_eq!(
        *app_ids.lock().expect("registry recording lock"),
        ["com.playervox.OverCrow"]
    );
    drop(client);
    drop(server);
    bus.shutdown().await;
}

#[tokio::test]
async fn isolated_bus_drop_terminates_and_reaps_its_child() {
    let bus = IsolatedSessionBus::start().await;
    let reaped = Arc::clone(&bus.process.reaped);

    drop(bus);

    assert!(reaped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn portal_absence_is_non_fatal() {
    let bus = IsolatedSessionBus::start().await;
    bus.assert_portal_is_not_activatable().await;
    let runtime = active_runtime().await;
    let portal = XdgPortal::for_address(bus.address.clone());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime, portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));

    wait_for_availability(
        &mut availability,
        |value| matches!(value, ShortcutAvailability::Unavailable(message) if !message.is_empty()),
    )
    .await;
    let ShortcutAvailability::Unavailable(message) = availability.borrow().clone() else {
        unreachable!();
    };
    assert!(message.len() <= ShortcutAvailability::MAX_MESSAGE_BYTES);
    assert!(!task.is_finished(), "portal absence must not stop Core");

    shutdown_tx.send_replace(true);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("broker shutdown timed out")
        .expect("broker task failed")
        .expect("idle broker shutdown failed");
    bus.shutdown().await;
}

#[test]
fn portal_tokens_and_predicted_request_paths_are_valid_and_unpredictable() {
    let first = random_portal_token("request").unwrap();
    let second = random_portal_token("request").unwrap();

    assert_ne!(first, second);
    assert!(first.starts_with("overcrow_request_"));
    assert!(
        first
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    );
    assert_eq!(
        request_path(":1.42", &first).unwrap().as_str(),
        format!("/org/freedesktop/portal/desktop/request/1_42/{first}")
    );
    assert!(request_path("not-a-unique-name", &first).is_err());
    assert!(request_path(":1.42", "bad/token").is_err());
}

#[test]
fn returned_request_handle_selects_only_the_verified_response_path() {
    let predicted = request_path(":1.42", "overcrow_request_a").unwrap();
    let legacy = "/org/freedesktop/portal/desktop/request/legacy/token";

    assert_eq!(
        request_path_strategy(&predicted, predicted.as_str()).unwrap(),
        RequestPathStrategy::Predicted
    );
    assert_eq!(
        request_path_strategy(&predicted, legacy).unwrap(),
        RequestPathStrategy::Legacy(legacy.try_into().unwrap())
    );
    assert!(request_path_strategy(&predicted, "not-an-object-path").is_err());
}

#[test]
fn create_response_requires_success_and_a_valid_session_handle() {
    let valid_handle = "/org/freedesktop/portal/desktop/session/1_42/overcrow_session_a";
    let expected = session_path(":1.42", "overcrow_session_a").unwrap();
    let valid = HashMap::from([(
        "session_handle".to_owned(),
        OwnedValue::from(Str::from(valid_handle)),
    )]);

    assert_eq!(
        parse_create_response(0, &valid, &expected)
            .unwrap()
            .as_str(),
        valid_handle
    );
    assert!(parse_create_response(1, &valid, &expected).is_err());
    assert!(parse_create_response(0, &HashMap::new(), &expected).is_err());
    let malformed = HashMap::from([(
        "session_handle".to_owned(),
        OwnedValue::from(Str::from("not-an-object-path")),
    )]);
    assert!(parse_create_response(0, &malformed, &expected).is_err());

    let wrong_but_valid = HashMap::from([(
        "session_handle".to_owned(),
        OwnedValue::from(Str::from(
            "/org/freedesktop/portal/desktop/session/1_42/another_session",
        )),
    )]);
    assert!(parse_create_response(0, &wrong_but_valid, &expected).is_err());
}

#[test]
fn bind_response_requires_exactly_the_requested_unique_shortcut_ids() {
    let requested = [
        "toggle-overlay",
        "toggle-manual-stopwatch",
        "reset-manual-stopwatch",
    ];
    let exact = requested
        .iter()
        .map(|id| ((*id).to_owned(), HashMap::new()))
        .collect::<Vec<_>>();
    let reordered = exact.iter().cloned().rev().collect::<Vec<_>>();
    let wrong = vec![("another-shortcut".to_owned(), HashMap::new())];
    let duplicate = vec![
        ("toggle-overlay".to_owned(), HashMap::new()),
        ("toggle-overlay".to_owned(), HashMap::new()),
    ];

    assert!(parse_bind_response(0, Some(&exact), &requested).is_ok());
    assert!(parse_bind_response(0, Some(&reordered), &requested).is_ok());
    assert!(parse_bind_response(1, Some(&exact), &requested).is_err());
    assert!(parse_bind_response(0, None, &requested).is_err());
    assert!(parse_bind_response(0, Some(&[]), &requested).is_err());
    assert!(parse_bind_response(0, Some(&wrong), &requested).is_err());
    assert!(parse_bind_response(0, Some(&duplicate), &requested[..2]).is_err());
    assert!(
        parse_bind_response(
            0,
            Some(&exact),
            &["toggle-overlay", "toggle-overlay", "reset-manual-stopwatch"]
        )
        .is_err()
    );
}

#[test]
fn bind_result_wrapper_rejects_missing_or_malformed_values() {
    let requested = ["toggle-overlay"];
    assert!(parse_bind_results(0, HashMap::new(), &requested).is_err());
    assert!(
        parse_bind_results(
            0,
            HashMap::from([(
                "shortcuts".to_owned(),
                OwnedValue::from(Str::from("not-an-array")),
            )]),
            &requested,
        )
        .is_err()
    );
    assert!(parse_bind_results(2, HashMap::new(), &requested).is_err());
}

struct FakeRequestEvents {
    events: mpsc::UnboundedReceiver<RequestEvent>,
    delivered: Arc<AtomicUsize>,
}

impl RequestEventSource for FakeRequestEvents {
    fn next_event(&mut self) -> ShortcutFuture<'_, Result<RequestEvent, ShortcutError>> {
        Box::pin(async {
            let event = self
                .events
                .recv()
                .await
                .ok_or_else(|| ShortcutError::new("fake request event stream closed"))?;
            self.delivered.fetch_add(1, Ordering::SeqCst);
            Ok(event)
        })
    }
}

fn response_event(source: ResponseSource, path: &str, marker: &str) -> RequestEvent {
    RequestEvent::Response {
        source,
        response: PortalResponse {
            path: path.try_into().unwrap(),
            code: 0,
            results: HashMap::from([("marker".to_owned(), OwnedValue::from(Str::from(marker)))]),
        },
    }
}

fn owner_change_event(old_owner: &str, new_owner: &str) -> RequestEvent {
    RequestEvent::OwnerChanged {
        old_owner: old_owner.to_owned(),
        new_owner: new_owner.to_owned(),
    }
}

#[tokio::test]
async fn immediate_legacy_response_is_buffered_before_the_method_returns() {
    let predicted = request_path(":1.42", "overcrow_request_predicted").unwrap();
    let legacy: OwnedObjectPath = "/org/freedesktop/portal/desktop/request/legacy/immediate"
        .try_into()
        .unwrap();
    let unrelated = "/org/freedesktop/portal/desktop/request/other/noise";
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let delivered = Arc::new(AtomicUsize::new(0));
    let source = FakeRequestEvents {
        events: event_rx,
        delivered: Arc::clone(&delivered),
    };
    let (method_tx, method_rx) = oneshot::channel();
    let expected_legacy = legacy.clone();
    let task = tokio::spawn(async move {
        await_monitored_response(source, predicted, async move {
            method_rx
                .await
                .map_err(|_| ShortcutError::new("fake method reply dropped"))
        })
        .await
    });

    event_tx
        .send(response_event(ResponseSource::Any, unrelated, "noise"))
        .unwrap();
    event_tx
        .send(response_event(
            ResponseSource::Any,
            expected_legacy.as_str(),
            "legacy",
        ))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while delivered.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("responses must be consumed before the method reply");
    method_tx.send(legacy).unwrap();

    let (_, results) = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("legacy response race must not hang")
        .unwrap()
        .unwrap();
    assert_eq!(
        <&str>::try_from(results.get("marker").unwrap()).unwrap(),
        "legacy"
    );
}

struct DropWitness(Arc<AtomicBool>);

impl Drop for DropWitness {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy)]
enum OwnerLossPhase {
    Create,
    Bind,
}

struct OwnerLossPortal {
    phase: OwnerLossPhase,
    events: Mutex<Option<mpsc::UnboundedReceiver<RequestEvent>>>,
    dropped: Arc<AtomicBool>,
    attempts: Arc<AtomicUsize>,
}

impl ShortcutPortal for OwnerLossPortal {
    fn bind(
        &self,
        _definitions: Vec<ShortcutDefinition>,
    ) -> ShortcutFuture<'static, Result<Box<dyn ShortcutSession>, ShortcutError>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let event_rx = self.events.lock().unwrap().take().unwrap();
        let source = FakeRequestEvents {
            events: event_rx,
            delivered: Arc::new(AtomicUsize::new(0)),
        };
        let phase = self.phase;
        let witness = DropWitness(Arc::clone(&self.dropped));
        Box::pin(async move {
            let _witness = witness;
            let predicted = request_path(":1.42", "overcrow_request_owner_loss")?;
            let returned = predicted.clone();
            match phase {
                OwnerLossPhase::Create => {
                    await_monitored_response(source, predicted, async move {
                        std::future::pending::<Result<OwnedObjectPath, ShortcutError>>().await
                    })
                    .await?;
                }
                OwnerLossPhase::Bind => {
                    await_monitored_response(source, predicted, async move { Ok(returned) })
                        .await?;
                }
            }
            Err(ShortcutError::new("owner-loss fixture unexpectedly bound"))
        })
    }
}

async fn assert_broker_handles_owner_change(phase: OwnerLossPhase, event: RequestEvent) {
    let runtime = active_runtime().await;
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(AtomicUsize::new(0));
    let portal = OwnerLossPortal {
        phase,
        events: Mutex::new(Some(event_rx)),
        dropped: Arc::clone(&dropped),
        attempts: Arc::clone(&attempts),
    };
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime.clone(), portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Binding)
    })
    .await;

    event_tx.send(event).unwrap();
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Unavailable(message) if message.contains("owner"))
    })
    .await;
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    runtime.set_overlay_interactive(true).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(!task.is_finished());

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn create_owner_loss_publishes_unavailable_and_drops_the_attempt() {
    assert_broker_handles_owner_change(OwnerLossPhase::Create, owner_change_event(":1.8", ""))
        .await;
}

#[tokio::test]
async fn bind_owner_loss_publishes_unavailable_and_drops_the_partial_session() {
    assert_broker_handles_owner_change(OwnerLossPhase::Bind, owner_change_event(":1.8", "")).await;
}

#[tokio::test]
async fn create_owner_replacement_publishes_unavailable_and_drops_the_attempt() {
    assert_broker_handles_owner_change(OwnerLossPhase::Create, owner_change_event(":1.8", ":1.9"))
        .await;
}

#[tokio::test]
async fn bind_owner_replacement_publishes_unavailable_and_drops_the_partial_session() {
    assert_broker_handles_owner_change(OwnerLossPhase::Bind, owner_change_event(":1.8", ":1.9"))
        .await;
}

#[tokio::test]
async fn owner_loss_cancels_a_pending_create_method_call() {
    let predicted = request_path(":1.42", "overcrow_request_create").unwrap();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let witness = DropWitness(Arc::clone(&dropped));
    let source = FakeRequestEvents {
        events: event_rx,
        delivered: Arc::new(AtomicUsize::new(0)),
    };
    let task = tokio::spawn(async move {
        await_monitored_response(source, predicted, async move {
            let _witness = witness;
            std::future::pending::<Result<OwnedObjectPath, ShortcutError>>().await
        })
        .await
    });

    event_tx.send(owner_change_event(":1.8", "")).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("owner loss must cancel CreateSession promptly")
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("owner"));
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn owner_loss_cancels_bind_while_waiting_for_its_response() {
    let predicted = request_path(":1.42", "overcrow_request_bind").unwrap();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let source = FakeRequestEvents {
        events: event_rx,
        delivered: Arc::new(AtomicUsize::new(0)),
    };
    let returned = predicted.clone();
    let task = tokio::spawn(async move {
        await_monitored_response(source, predicted, async move { Ok(returned) }).await
    });

    tokio::task::yield_now().await;
    event_tx.send(owner_change_event(":1.8", "")).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("owner loss must cancel BindShortcuts response wait promptly")
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("owner"));
}

#[tokio::test]
async fn owner_appearance_for_auto_activation_does_not_cancel_the_request() {
    let predicted = request_path(":1.42", "overcrow_request_activation").unwrap();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let source = FakeRequestEvents {
        events: event_rx,
        delivered: Arc::new(AtomicUsize::new(0)),
    };
    let returned = predicted.clone();
    let expected = predicted.clone();
    let task = tokio::spawn(async move {
        await_monitored_response(source, predicted, async move { Ok(returned) }).await
    });

    event_tx.send(owner_change_event("", ":1.8")).unwrap();
    event_tx
        .send(response_event(
            ResponseSource::Predicted,
            expected.as_str(),
            "activated",
        ))
        .unwrap();
    let (_, results) = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("owner acquisition must not cancel auto-activated portal request")
        .unwrap()
        .unwrap();
    assert_eq!(
        <&str>::try_from(results.get("marker").unwrap()).unwrap(),
        "activated"
    );
}

#[tokio::test]
async fn live_owner_replacement_invalidates_the_available_session() {
    let runtime = active_runtime().await;
    let (portal, mut attempts, state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime, portal);
    let mut availability = broker.availability();
    let task = tokio::spawn(broker.run(shutdown_rx));
    let events = attempts
        .recv()
        .await
        .unwrap()
        .respond_session("/session/replaced-owner");
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Available)
    })
    .await;

    let event = portal_owner_change_event(":1.8", ":1.9")
        .unwrap()
        .expect("direct replacement must invalidate the live portal session");
    events.send(event).unwrap();
    wait_for_availability(&mut availability, |value| {
        matches!(value, ShortcutAvailability::Unavailable(message) if message.contains("owner"))
    })
    .await;
    assert_eq!(state.closes.load(Ordering::SeqCst), 1);
    assert_eq!(state.live_sessions.load(Ordering::SeqCst), 0);

    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn core1_exposes_the_live_bounded_shortcut_availability() {
    let bus = IsolatedSessionBus::start().await;
    let runtime = active_runtime().await;
    let service = CoreService::with_runtime(runtime.clone());
    let _server = zbus::connection::Builder::address(bus.address.clone())
        .unwrap()
        .name("io.github.overcrow.Core1")
        .unwrap()
        .serve_at("/io/github/overcrow/Core1", service)
        .unwrap()
        .build()
        .await
        .unwrap();
    let client = zbus::connection::Builder::address(bus.address.clone())
        .unwrap()
        .build()
        .await
        .unwrap();
    let service = Core1Proxy::new(&client).await.unwrap();
    let (portal, mut attempts, _state) = fake_portal();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let broker = PortalShortcutBroker::with_portal(runtime, portal);
    let task = tokio::spawn(broker.run(shutdown_rx));

    attempts
        .recv()
        .await
        .unwrap()
        .deny(&"x".repeat(ShortcutAvailability::MAX_MESSAGE_BYTES * 2));
    let diagnostic = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let value = service.shortcut_availability().await.unwrap();
            if value.starts_with("unavailable: ") {
                break value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Core1 diagnostic must observe broker state");

    assert!(diagnostic.len() <= ShortcutAvailability::MAX_DIAGNOSTIC_BYTES);
    shutdown_tx.send_replace(true);
    task.await.unwrap().unwrap();
    bus.shutdown().await;
}
