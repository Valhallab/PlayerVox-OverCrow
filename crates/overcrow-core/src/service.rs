use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use overcrow_config::{SettingsLoad, SettingsStore, WidgetSettingsLoad, WidgetSettingsStore};
use overcrow_protocol::{CoreSnapshot, CoreState, Rect, VersionedCoreSnapshot};
use tokio::sync::RwLock;
use tokio::time::{MissedTickBehavior, interval};

use crate::{CoreRuntime, OVERLAY_APP_ID, WindowObservation, WindowSource};

pub const WINDOW_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const WIDGET_SETTINGS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct CoreService {
    runtime: CoreRuntime,
    settings_loader: Option<Arc<dyn SettingsLoader>>,
    widget_settings_loader: Option<Arc<dyn WidgetSettingsLoader>>,
    reload_transaction: Arc<tokio::sync::Mutex<()>>,
    blocking_runtime: Option<tokio::runtime::Handle>,
}

trait SettingsLoader: Send + Sync {
    fn load(&self) -> SettingsLoad;
}

trait WidgetSettingsLoader: Send + Sync {
    fn load(&self) -> WidgetSettingsLoad;
}

impl SettingsLoader for SettingsStore {
    fn load(&self) -> SettingsLoad {
        SettingsStore::load(self)
    }
}

impl WidgetSettingsLoader for WidgetSettingsStore {
    fn load(&self) -> WidgetSettingsLoad {
        WidgetSettingsStore::load(self)
    }
}

impl CoreService {
    pub async fn new(state: Arc<RwLock<CoreState>>) -> Self {
        Self::with_runtime(CoreRuntime::new(state, HashMap::new()).await)
    }

    pub fn with_runtime(runtime: CoreRuntime) -> Self {
        Self {
            runtime,
            settings_loader: None,
            widget_settings_loader: None,
            reload_transaction: Arc::new(tokio::sync::Mutex::new(())),
            blocking_runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    pub fn with_runtime_and_store(
        runtime: CoreRuntime,
        settings_store: Arc<SettingsStore>,
    ) -> Self {
        Self {
            runtime,
            settings_loader: Some(settings_store),
            widget_settings_loader: None,
            reload_transaction: Arc::new(tokio::sync::Mutex::new(())),
            blocking_runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    pub fn with_runtime_and_widget_store(
        runtime: CoreRuntime,
        widget_settings_store: Arc<WidgetSettingsStore>,
    ) -> Self {
        Self {
            runtime,
            settings_loader: None,
            widget_settings_loader: Some(widget_settings_store),
            reload_transaction: Arc::new(tokio::sync::Mutex::new(())),
            blocking_runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    pub fn with_runtime_and_stores(
        runtime: CoreRuntime,
        settings_store: Arc<SettingsStore>,
        widget_settings_store: Arc<WidgetSettingsStore>,
    ) -> Self {
        Self {
            runtime,
            settings_loader: Some(settings_store),
            widget_settings_loader: Some(widget_settings_store),
            reload_transaction: Arc::new(tokio::sync::Mutex::new(())),
            blocking_runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    #[cfg(test)]
    fn with_runtime_and_loader<L>(runtime: CoreRuntime, settings_loader: Arc<L>) -> Self
    where
        L: SettingsLoader + 'static,
    {
        Self {
            runtime,
            settings_loader: Some(settings_loader),
            widget_settings_loader: None,
            reload_transaction: Arc::new(tokio::sync::Mutex::new(())),
            blocking_runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    fn snapshot_json(snapshot: &CoreSnapshot) -> zbus::fdo::Result<String> {
        serde_json::to_string(snapshot).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    pub(crate) fn versioned_snapshot_json(
        snapshot: &VersionedCoreSnapshot,
    ) -> zbus::fdo::Result<String> {
        serde_json::to_string(snapshot).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::future::Future;
    use std::path::PathBuf;
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Waker};
    use std::thread;
    use std::time::Duration;
    use std::time::Instant;

    use overcrow_config::{LifecycleSettings, SettingsLoad, WidgetProfile, WidgetSettingsStore};
    use overcrow_protocol::CoreState;
    use tokio::sync::RwLock;

    use super::{CoreService, SettingsLoader, run_widget_settings_refresh_at};
    use crate::{CoreRuntime, ProcessInfo};

    fn block_on_without_tokio<F: Future>(future: F) -> F::Output {
        let mut future = Box::pin(future);
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1));
                }
                Poll::Pending => panic!("future did not complete outside a Tokio executor"),
            }
        }
    }

    #[test]
    fn widget_reload_uses_the_captured_runtime_from_a_foreign_executor() {
        let tokio = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("Tokio runtime");
        let temp = tempfile::tempdir().expect("create test directory");
        let store = Arc::new(WidgetSettingsStore::from_paths(
            temp.path().join("widgets.json"),
            temp.path().join("overlay.json"),
        ));
        store
            .save(&WidgetProfile::default())
            .expect("seed private profile");
        let service = tokio.block_on(async {
            let runtime =
                CoreRuntime::new(Arc::new(RwLock::new(CoreState::default())), HashMap::new()).await;
            CoreService::with_runtime_and_widget_store(runtime, store)
        });

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            block_on_without_tokio(service.reload_widget_settings())
        }));

        result
            .expect("reload must not require the caller to be a Tokio executor")
            .expect("widget reload succeeds");
    }

    #[tokio::test]
    async fn concurrent_reload_transactions_cannot_apply_an_older_load_last() {
        let runtime = CoreRuntime::with_settings(
            Arc::new(RwLock::new(CoreState::default())),
            HashMap::from([(42, steam_process(620))]),
            enabled_settings_with_steam([620]),
        )
        .await;
        let loader = Arc::new(ControllableLoader::new(enabled_settings_with_steam([620])));
        let service = CoreService::with_runtime_and_loader(runtime.clone(), loader.clone());

        let first_service = service.clone();
        let first = tokio::spawn(async move { first_service.reload_settings().await });
        loader.wait_until_first_load_is_blocked().await;
        loader.replace(enabled_settings_with_steam([730]));

        let second_service = service.clone();
        assert!(Arc::ptr_eq(
            &service.reload_transaction,
            &second_service.reload_transaction
        ));
        let (second_started_tx, second_started_rx) = tokio::sync::oneshot::channel();
        let second = tokio::spawn(async move {
            second_started_tx.send(()).expect("signal second reload");
            second_service.reload_settings().await
        });
        second_started_rx.await.expect("second reload started");

        assert!(service.reload_transaction.try_lock().is_err());
        assert_eq!(loader.calls(), 1);
        loader.release_first_load();

        first
            .await
            .expect("first reload task")
            .expect("first reload");
        second
            .await
            .expect("second reload task")
            .expect("second reload");

        assert_eq!(loader.calls(), 2);
        assert!(!*runtime.selected_processes_running().borrow());
    }

    #[tokio::test]
    async fn periodic_widget_refresh_recovers_from_warning_without_unchanged_watch_noise() {
        let runtime =
            CoreRuntime::new(Arc::new(RwLock::new(CoreState::default())), HashMap::new()).await;
        let mut profiles = runtime.widget_profile();
        let temp = tempfile::tempdir().expect("create test directory");
        let path = temp.path().join("widgets.json");
        let store = Arc::new(WidgetSettingsStore::from_paths(
            &path,
            temp.path().join("overlay.json"),
        ));
        store
            .save(&WidgetProfile::default())
            .expect("seed private profile");
        std::fs::write(&path, b"not json").expect("replace with malformed profile");
        let service = CoreService::with_runtime_and_widget_store(runtime, Arc::clone(&store));
        let refresh = tokio::spawn(run_widget_settings_refresh_at(
            service,
            Duration::from_millis(10),
        ));
        tokio::time::sleep(Duration::from_millis(20)).await;

        let mut enabled = WidgetProfile::default();
        enabled.manual_stopwatch.enabled = true;
        store.save(&enabled).expect("replace with valid profile");

        tokio::time::timeout(Duration::from_millis(100), profiles.changed())
            .await
            .expect("periodic refresh applies recovered profile")
            .expect("profile watch remains open");
        assert_eq!(*profiles.borrow_and_update(), enabled);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), profiles.changed())
                .await
                .is_err()
        );

        refresh.abort();
        let _ = refresh.await;
    }

    struct ControllableLoader {
        state: Mutex<LoaderState>,
        first_blocked: tokio::sync::Notify,
        release_first: Condvar,
    }

    struct LoaderState {
        current: LifecycleSettings,
        calls: usize,
        first_released: bool,
    }

    impl ControllableLoader {
        fn new(current: LifecycleSettings) -> Self {
            Self {
                state: Mutex::new(LoaderState {
                    current,
                    calls: 0,
                    first_released: false,
                }),
                first_blocked: tokio::sync::Notify::new(),
                release_first: Condvar::new(),
            }
        }

        async fn wait_until_first_load_is_blocked(&self) {
            loop {
                let notified = self.first_blocked.notified();
                if self.state.lock().expect("loader state").calls > 0 {
                    return;
                }
                notified.await;
            }
        }

        fn replace(&self, settings: LifecycleSettings) {
            self.state.lock().expect("loader state").current = settings;
        }

        fn calls(&self) -> usize {
            self.state.lock().expect("loader state").calls
        }

        fn release_first_load(&self) {
            let mut state = self.state.lock().expect("loader state");
            state.first_released = true;
            self.release_first.notify_all();
        }
    }

    impl SettingsLoader for ControllableLoader {
        fn load(&self) -> SettingsLoad {
            let mut state = self.state.lock().expect("loader state");
            state.calls += 1;
            let captured = state.current.clone();
            if state.calls == 1 {
                self.first_blocked.notify_waiters();
                while !state.first_released {
                    state = self.release_first.wait(state).expect("loader state");
                }
            }
            SettingsLoad {
                settings: captured,
                warning: None,
            }
        }
    }

    fn enabled_settings_with_steam<const N: usize>(ids: [u32; N]) -> LifecycleSettings {
        LifecycleSettings {
            enabled: true,
            selected_steam_app_ids: BTreeSet::from(ids),
            ..LifecycleSettings::default()
        }
    }

    fn steam_process(app_id: u32) -> ProcessInfo {
        ProcessInfo {
            pid: 42,
            parent_pid: 1,
            start_ticks: 0,
            timing: None,
            resources: Default::default(),
            name: "game".to_owned(),
            environment: HashMap::from([("SteamAppId".to_owned(), app_id.to_string())]),
            command_line: Vec::new(),
            executable: Some(PathBuf::from("/games/game")),
        }
    }
}

pub async fn apply_window_observation(
    runtime: &CoreRuntime,
    observation: Option<WindowObservation>,
) {
    runtime.apply_x11_observation(observation).await;
}

pub async fn poll_window_once<S: WindowSource + ?Sized>(
    source: &mut S,
    runtime: &CoreRuntime,
) -> anyhow::Result<()> {
    let observation = source.active_window()?;
    apply_window_observation(runtime, observation).await;
    Ok(())
}

pub fn should_use_x11_source(session_type: Option<&str>) -> bool {
    session_type.is_some_and(|session| session.eq_ignore_ascii_case("x11"))
}

pub async fn run_window_polling<S: WindowSource>(mut source: S, runtime: CoreRuntime) {
    let mut window_poll = interval(WINDOW_POLL_INTERVAL);
    window_poll.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        window_poll.tick().await;
        if let Err(error) = poll_window_once(&mut source, &runtime).await {
            runtime.clear_game().await;
            eprintln!("OverCrow X11 poll failed: {error:#}");
        }
    }
}

pub async fn run_widget_settings_refresh(service: CoreService) {
    run_widget_settings_refresh_at(service, WIDGET_SETTINGS_REFRESH_INTERVAL).await;
}

async fn run_widget_settings_refresh_at(service: CoreService, refresh_interval: Duration) {
    let mut refresh = interval(refresh_interval);
    refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
    refresh.tick().await;

    loop {
        refresh.tick().await;
        if let Err(error) = service.reload_widget_settings().await {
            eprintln!("OverCrow widget settings refresh failed: {error}");
        }
    }
}

#[zbus::interface(name = "io.github.overcrow.Core1")]
impl CoreService {
    #[zbus(name = "Snapshot")]
    pub async fn snapshot(&self) -> zbus::fdo::Result<String> {
        Self::snapshot_json(&self.runtime.snapshot().await)
    }

    #[zbus(name = "SnapshotVersioned")]
    pub async fn snapshot_versioned(&self) -> zbus::fdo::Result<String> {
        Self::versioned_snapshot_json(&self.runtime.versioned_snapshot())
    }

    #[zbus(signal, name = "SnapshotChanged")]
    pub async fn snapshot_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        snapshot_json: &str,
    ) -> zbus::Result<()>;

    #[zbus(name = "ToggleOverlay")]
    pub async fn toggle_overlay(&self) -> zbus::fdo::Result<String> {
        Self::snapshot_json(&self.runtime.toggle_overlay().await)
    }

    #[zbus(name = "SetOverlayInteractive")]
    pub async fn set_overlay_interactive(&self, interactive: bool) -> zbus::fdo::Result<String> {
        Self::snapshot_json(&self.runtime.set_overlay_interactive(interactive).await)
    }

    #[zbus(name = "ReportWindow")]
    #[allow(clippy::too_many_arguments)]
    pub async fn report_window(
        &self,
        pid: i32,
        title: &str,
        app_id: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        scale: &str,
    ) -> zbus::fdo::Result<String> {
        if app_id == OVERLAY_APP_ID {
            return self.snapshot().await;
        }
        if pid <= 0 || width <= 0 || height <= 0 {
            return Self::snapshot_json(&self.runtime.clear_game().await);
        }
        let (pid, width, height) = match (
            u32::try_from(pid),
            u32::try_from(width),
            u32::try_from(height),
        ) {
            (Ok(pid), Ok(width), Ok(height)) => (pid, width, height),
            _ => return Self::snapshot_json(&self.runtime.clear_game().await),
        };
        let scale = match scale.parse::<f64>() {
            Ok(scale) if scale.is_finite() && scale > 0.0 => scale,
            _ => return Self::snapshot_json(&self.runtime.clear_game().await),
        };
        let observation = WindowObservation {
            pid: Some(pid),
            app_id: (!app_id.is_empty()).then(|| app_id.to_owned()),
            title: title.to_owned(),
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            scale,
            backend: "wayland".to_owned(),
        };
        let snapshot = self.runtime.apply_bridge_observation(observation).await;
        Self::snapshot_json(&snapshot)
    }

    #[zbus(name = "ClearWindow")]
    pub async fn clear_window(&self) -> zbus::fdo::Result<String> {
        let snapshot = self.runtime.clear_game().await;
        Self::snapshot_json(&snapshot)
    }

    #[zbus(name = "ReloadSettings")]
    pub async fn reload_settings(&self) -> zbus::fdo::Result<String> {
        let _transaction = self.reload_transaction.lock().await;
        let loader = self.settings_loader.clone().ok_or_else(|| {
            zbus::fdo::Error::Failed("lifecycle settings store is unavailable".to_owned())
        })?;
        let blocking_runtime = self.blocking_runtime.as_ref().ok_or_else(|| {
            zbus::fdo::Error::Failed("Core blocking runtime is unavailable".to_owned())
        })?;
        let load = blocking_runtime
            .spawn_blocking(move || loader.load())
            .await
            .map_err(|error| {
                zbus::fdo::Error::Failed(format!("lifecycle settings load failed: {error}"))
            })?;
        if let Some(warning) = load.warning {
            return Err(zbus::fdo::Error::Failed(warning));
        }
        let snapshot = self
            .runtime
            .reload_settings(load.settings)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        Self::snapshot_json(&snapshot)
    }

    #[zbus(name = "ReloadWidgetSettings")]
    pub async fn reload_widget_settings(&self) -> zbus::fdo::Result<String> {
        let _transaction = self.reload_transaction.lock().await;
        let loader = self.widget_settings_loader.clone().ok_or_else(|| {
            zbus::fdo::Error::Failed("widget settings store is unavailable".to_owned())
        })?;
        let blocking_runtime = self.blocking_runtime.as_ref().ok_or_else(|| {
            zbus::fdo::Error::Failed("Core blocking runtime is unavailable".to_owned())
        })?;
        let load = blocking_runtime
            .spawn_blocking(move || loader.load())
            .await
            .map_err(|error| {
                zbus::fdo::Error::Failed(format!("widget settings load failed: {error}"))
            })?;
        if let Some(warning) = load.warning {
            return Err(zbus::fdo::Error::Failed(warning));
        }
        let snapshot = self
            .runtime
            .reload_widget_profile(load.profile)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        Self::snapshot_json(&snapshot)
    }

    #[zbus(name = "ToggleManualStopwatch")]
    pub async fn toggle_manual_stopwatch(&self) -> zbus::fdo::Result<String> {
        Self::snapshot_json(
            &self
                .runtime
                .toggle_manual_stopwatch_at(std::time::Instant::now())
                .await,
        )
    }

    #[zbus(name = "ResetManualStopwatch")]
    pub async fn reset_manual_stopwatch(&self) -> zbus::fdo::Result<String> {
        Self::snapshot_json(
            &self
                .runtime
                .reset_manual_stopwatch_at(std::time::Instant::now())
                .await,
        )
    }

    #[zbus(name = "ShortcutAvailability")]
    pub async fn shortcut_availability(&self) -> zbus::fdo::Result<String> {
        Ok(self.runtime.shortcut_availability_diagnostic())
    }
}
