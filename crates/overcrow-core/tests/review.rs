use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overcrow_config::{GameAllowlist, LifecycleSettings};
use overcrow_core::{
    BRIDGE_LEASE_TIMEOUT, CoreRuntime, CoreService, OVERLAY_APP_ID, ProcessInfo, WindowObservation,
    classify_process_identity,
};
use overcrow_protocol::{CoreSnapshot, CoreState, GameWindow, OverlayMode, Rect};
use tokio::sync::RwLock;

#[tokio::test]
async fn report_window_uses_the_shared_process_snapshot() {
    let (runtime, _state) = runtime_with(HashMap::from([(42, sample_process())])).await;
    let service = CoreService::with_runtime(runtime);

    let snapshot = report(&service, 42, "portal2", "1.25").await;

    assert_eq!(snapshot.active_game, Some(sample_wayland_game()));
}

#[tokio::test]
async fn report_window_rejects_a_pid_missing_from_the_process_snapshot() {
    let (runtime, state) = runtime_with(HashMap::new()).await;
    state.write().await.observe_game(sample_wayland_game());
    state.write().await.toggle_overlay();
    let service = CoreService::with_runtime(runtime);

    let snapshot = report(&service, 99, "org.example.Browser", "1").await;

    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[tokio::test]
async fn report_window_ignores_the_overlay_app_id() {
    let (runtime, state) = runtime_with(HashMap::from([(42, sample_process())])).await;
    state.write().await.observe_game(sample_wayland_game());
    state.write().await.toggle_overlay();
    let service = CoreService::with_runtime(runtime);

    let snapshot = report(&service, 42, OVERLAY_APP_ID, "1").await;

    assert_eq!(snapshot.active_game, Some(sample_wayland_game()));
    assert_eq!(snapshot.overlay_mode, OverlayMode::Interactive);
}

#[tokio::test]
async fn process_refresh_clears_an_active_game_when_its_pid_disappears() {
    let (runtime, state) = runtime_with(HashMap::from([(42, sample_process())])).await;
    state.write().await.observe_game(sample_wayland_game());
    state.write().await.toggle_overlay();

    runtime.install_process_snapshot(HashMap::new()).await;

    let snapshot = state.read().await.snapshot().clone();
    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[tokio::test]
async fn process_refresh_clears_a_reused_pid_with_a_different_steam_id() {
    let (runtime, state) = runtime_with(HashMap::from([(42, sample_process())])).await;
    state.write().await.observe_game(sample_wayland_game());
    state.write().await.toggle_overlay();
    let mut replacement = sample_process();
    replacement
        .environment
        .insert("SteamAppId".to_owned(), "730".to_owned());

    runtime
        .install_process_snapshot(HashMap::from([(42, replacement)]))
        .await;

    let snapshot = state.read().await.snapshot().clone();
    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[tokio::test]
async fn bridge_lease_expires_at_a_controllable_deadline() {
    let (runtime, state) = runtime_with(HashMap::from([(42, sample_process())])).await;
    let reported_at = Instant::now();
    runtime
        .apply_bridge_observation_at(sample_wayland_observation(), reported_at)
        .await;
    state.write().await.toggle_overlay();

    let expired_early = runtime
        .expire_bridge_lease_at(reported_at + BRIDGE_LEASE_TIMEOUT - Duration::from_millis(1))
        .await;
    assert!(!expired_early);
    assert_eq!(
        state.read().await.snapshot().overlay_mode,
        OverlayMode::Interactive
    );

    let expired_at_deadline = runtime
        .expire_bridge_lease_at(reported_at + BRIDGE_LEASE_TIMEOUT)
        .await;
    assert!(expired_at_deadline);
    let snapshot = state.read().await.snapshot().clone();
    assert_eq!(snapshot.active_game, None);
    assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
}

#[test]
fn selected_process_scan_reclassifies_a_reused_pid_from_the_current_snapshot() {
    let allowlist = GameAllowlist::from_settings(&enabled_settings_with_steam([620]));
    let mut processes = HashMap::from([(42, sample_process())]);

    assert!(snapshot_has_selected_process(&allowlist, &processes));

    let mut replacement = sample_process();
    replacement
        .environment
        .insert("SteamAppId".to_owned(), "730".to_owned());
    replacement.executable = Some(PathBuf::from("/games/unselected"));
    processes.insert(42, replacement);

    assert!(!snapshot_has_selected_process(&allowlist, &processes));
}

#[test]
fn selected_process_scan_checks_all_simultaneous_process_identities() {
    let allowlist = GameAllowlist::from_settings(&enabled_settings_with_steam([620]));
    let mut unselected = sample_process();
    unselected.pid = 7;
    unselected
        .environment
        .insert("SteamAppId".to_owned(), "730".to_owned());
    let processes = HashMap::from([(7, unselected), (42, sample_process())]);

    assert!(snapshot_has_selected_process(&allowlist, &processes));
}

#[tokio::test]
async fn report_window_rejects_non_finite_or_non_positive_scale() {
    let (runtime, state) = runtime_with(HashMap::from([(42, sample_process())])).await;
    let service = CoreService::with_runtime(runtime);

    for scale in ["", "not-a-number", "NaN", "inf", "-inf", "0", "-1"] {
        make_interactive(&state).await;

        let snapshot = report(&service, 42, "portal2", scale).await;

        assert_eq!(snapshot.active_game, None, "scale {scale:?}");
        assert_eq!(
            snapshot.overlay_mode,
            OverlayMode::Passive,
            "scale {scale:?}"
        );
    }
}

#[tokio::test]
async fn report_window_parses_integer_and_fractional_scale_strings() {
    let (runtime, _state) = runtime_with(HashMap::from([(42, sample_process())])).await;
    let service = CoreService::with_runtime(runtime);

    for (wire_scale, expected) in [("1", 1.0), ("1.25", 1.25)] {
        let snapshot = report(&service, 42, "portal2", wire_scale).await;
        assert_eq!(snapshot.active_game.expect("active game").scale, expected);
    }
}

#[tokio::test]
async fn report_window_rejects_zero_sized_geometry() {
    let (runtime, state) = runtime_with(HashMap::from([(42, sample_process())])).await;
    let service = CoreService::with_runtime(runtime);

    for (width, height) in [(0, 1080), (1920, 0)] {
        make_interactive(&state).await;
        let snapshot = report_geometry(&service, width, height).await;

        assert_eq!(snapshot.active_game, None, "geometry {width}x{height}");
        assert_eq!(
            snapshot.overlay_mode,
            OverlayMode::Passive,
            "geometry {width}x{height}"
        );
    }
}

#[tokio::test]
async fn report_window_rejects_non_positive_signed_transport_values() {
    let (runtime, state) = runtime_with(HashMap::from([(42, sample_process())])).await;
    let service = CoreService::with_runtime(runtime);

    for (pid, width, height) in [
        (-1, 1920, 1080),
        (0, 1920, 1080),
        (42, -1, 1080),
        (42, 1920, -1),
    ] {
        make_interactive(&state).await;
        let json = service
            .report_window(pid, "Portal 2", "portal2", -100, 24, width, height, "1")
            .await
            .expect("invalid signed transport values should fail closed");
        let snapshot: CoreSnapshot = serde_json::from_str(&json).expect("valid snapshot JSON");

        assert_eq!(
            snapshot.active_game, None,
            "transport ({pid}, {width}, {height})"
        );
        assert_eq!(snapshot.overlay_mode, OverlayMode::Passive);
    }
}

async fn report(service: &CoreService, pid: i32, app_id: &str, scale: &str) -> CoreSnapshot {
    let json = service
        .report_window(pid, "Portal 2", app_id, -100, 24, 1920, 1080, scale)
        .await
        .expect("in-memory state should serialize");
    serde_json::from_str(&json).expect("valid snapshot JSON")
}

async fn report_geometry(service: &CoreService, width: i32, height: i32) -> CoreSnapshot {
    let json = service
        .report_window(42, "Portal 2", "portal2", -100, 24, width, height, "1")
        .await
        .expect("invalid geometry should fail closed without serialization failure");
    serde_json::from_str(&json).expect("valid snapshot JSON")
}

async fn make_interactive(state: &Arc<RwLock<CoreState>>) {
    let mut state = state.write().await;
    state.observe_game(sample_wayland_game());
    state.set_overlay_interactive(true);
}

async fn runtime_with(
    processes: HashMap<u32, ProcessInfo>,
) -> (CoreRuntime, Arc<RwLock<CoreState>>) {
    let state = Arc::new(RwLock::new(CoreState::default()));
    (
        CoreRuntime::with_settings(
            Arc::clone(&state),
            processes,
            enabled_settings_with_steam([620]),
        )
        .await,
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

fn enabled_settings_with_steam<const N: usize>(ids: [u32; N]) -> LifecycleSettings {
    LifecycleSettings {
        enabled: true,
        selected_steam_app_ids: BTreeSet::from(ids),
        ..LifecycleSettings::default()
    }
}

fn snapshot_has_selected_process(
    allowlist: &GameAllowlist,
    processes: &HashMap<u32, ProcessInfo>,
) -> bool {
    allowlist.any_selected_process(processes.keys().copied(), |pid| {
        classify_process_identity(pid, processes)
    })
}

fn sample_wayland_observation() -> WindowObservation {
    WindowObservation {
        pid: Some(42),
        app_id: Some("portal2".to_owned()),
        title: "Portal 2".to_owned(),
        rect: Rect {
            x: -100,
            y: 24,
            width: 1920,
            height: 1080,
        },
        scale: 1.25,
        backend: "wayland".to_owned(),
    }
}

fn sample_wayland_game() -> GameWindow {
    GameWindow {
        pid: Some(42),
        steam_app_id: Some(620),
        app_id: Some("portal2".to_owned()),
        title: "Portal 2".to_owned(),
        rect: Rect {
            x: -100,
            y: 24,
            width: 1920,
            height: 1080,
        },
        scale: 1.25,
        backend: "wayland".to_owned(),
    }
}
