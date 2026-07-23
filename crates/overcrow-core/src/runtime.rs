use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overcrow_config::{
    GameAllowlist, LifecycleSettings, SettingsError, ShortcutSettings, WidgetProfile,
    WidgetProfileError,
};
use overcrow_protocol::{
    CoreSnapshot, CoreState, GameTelemetry, GameWindow, ManualStopwatchSnapshot,
    VersionedCoreSnapshot,
};
use tokio::sync::{Mutex, RwLock, watch};

use crate::{
    ManualStopwatch, ProcessClassification, ProcessIdentity, ProcessInfo, ProcessTiming,
    ShortcutAvailability, TelemetrySampler, TemperatureSnapshot, WindowObservation,
    classify_process_identity, collect_process_sample, scan_processes, scan_temperatures,
};

pub const BRIDGE_LEASE_TIMEOUT: Duration = Duration::from_secs(5);
pub const BRIDGE_WATCHDOG_INTERVAL: Duration = Duration::from_millis(250);
pub const PROCESS_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
pub const OVERLAY_APP_ID: &str = "io.github.overcrow.Overlay";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeSettingsError {
    Disabled,
    Invalid(SettingsError),
}

impl fmt::Display for RuntimeSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("lifecycle settings are disabled"),
            Self::Invalid(error) => write!(formatter, "invalid lifecycle settings: {error}"),
        }
    }
}

impl Error for RuntimeSettingsError {}

#[derive(Default)]
struct RuntimeMetadata {
    bridge_last_report: Option<Instant>,
    active_identity: Option<ProcessIdentity>,
    active_process: Option<ProcessInstance>,
    active_timing: Option<ProcessTiming>,
    timing_floor: Option<ProcessTiming>,
    telemetry: Option<GameTelemetry>,
    telemetry_sampler: TelemetrySampler,
    manual_stopwatch: ManualStopwatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessInstance {
    pid: u32,
    start_ticks: u64,
}

struct AllowedProcess {
    identity: ProcessIdentity,
    instance: ProcessInstance,
    timing: Option<ProcessTiming>,
}

#[derive(Clone)]
pub struct CoreRuntime {
    state: Arc<RwLock<CoreState>>,
    processes: Arc<RwLock<HashMap<u32, ProcessInfo>>>,
    settings: Arc<RwLock<LifecycleSettings>>,
    widget_profile: Arc<RwLock<WidgetProfile>>,
    snapshot_tx: watch::Sender<VersionedCoreSnapshot>,
    selected_running_tx: watch::Sender<bool>,
    shortcut_settings_tx: watch::Sender<ShortcutSettings>,
    widget_profile_tx: watch::Sender<WidgetProfile>,
    shortcut_availability_tx: watch::Sender<ShortcutAvailability>,
    metadata: Arc<Mutex<RuntimeMetadata>>,
    mutation: Arc<Mutex<()>>,
}

impl CoreRuntime {
    // Mutations serialize first, then acquire settings/widget profile/processes, metadata, and state.
    /// Constructs an inert runtime. Production callers should use `with_settings`.
    pub async fn new(state: Arc<RwLock<CoreState>>, processes: HashMap<u32, ProcessInfo>) -> Self {
        Self::with_settings(state, processes, LifecycleSettings::default()).await
    }

    pub async fn with_settings(
        state: Arc<RwLock<CoreState>>,
        processes: HashMap<u32, ProcessInfo>,
        settings: LifecycleSettings,
    ) -> Self {
        Self::with_settings_and_widget_profile(state, processes, settings, WidgetProfile::default())
            .await
    }

    pub async fn with_settings_and_widget_profile(
        state: Arc<RwLock<CoreState>>,
        processes: HashMap<u32, ProcessInfo>,
        settings: LifecycleSettings,
        widget_profile: WidgetProfile,
    ) -> Self {
        let settings = validate_enabled_settings(settings).unwrap_or_default();
        let widget_profile = widget_profile.validate().unwrap_or_default();
        let allowlist = GameAllowlist::from_settings(&settings);
        let selected_running = snapshot_has_selected_process(&allowlist, &processes);

        let (mut initial_snapshot, active_identity, active_process, active_timing) = {
            let mut state = state.write().await;
            let active_identity = state
                .snapshot()
                .active_game
                .as_ref()
                .and_then(|game| matching_process_identity(game, &processes, &allowlist));
            let active_process_timing = active_identity.as_ref().and_then(|_| {
                state
                    .snapshot()
                    .active_game
                    .as_ref()
                    .and_then(|game| active_game_process(game, &processes))
            });
            let (active_process, active_timing) = active_process_timing
                .map_or((None, None), |(instance, timing)| (Some(instance), timing));
            if active_identity.is_none() {
                state.clear_game();
            }
            (
                state.snapshot().clone(),
                active_identity,
                active_process,
                active_timing,
            )
        };
        initial_snapshot = snapshot_with_runtime(
            initial_snapshot,
            active_timing,
            None,
            ManualStopwatchSnapshot::default(),
            Instant::now(),
        );
        let (snapshot_tx, _) = watch::channel(VersionedCoreSnapshot {
            revision: 0,
            snapshot: initial_snapshot,
        });
        let (selected_running_tx, _) = watch::channel(selected_running);
        let (shortcut_settings_tx, _) = watch::channel(settings.shortcut.clone());
        let (widget_profile_tx, _) = watch::channel(widget_profile.clone());
        let (shortcut_availability_tx, _) = watch::channel(ShortcutAvailability::Disabled);

        Self {
            state,
            processes: Arc::new(RwLock::new(processes)),
            settings: Arc::new(RwLock::new(settings)),
            widget_profile: Arc::new(RwLock::new(widget_profile)),
            snapshot_tx,
            selected_running_tx,
            shortcut_settings_tx,
            widget_profile_tx,
            shortcut_availability_tx,
            metadata: Arc::new(Mutex::new(RuntimeMetadata {
                bridge_last_report: None,
                active_identity,
                active_process,
                active_timing,
                timing_floor: active_timing,
                telemetry: None,
                telemetry_sampler: TelemetrySampler::default(),
                manual_stopwatch: ManualStopwatch::default(),
            })),
            mutation: Arc::new(Mutex::new(())),
        }
    }

    pub fn snapshots(&self) -> watch::Receiver<VersionedCoreSnapshot> {
        self.snapshot_tx.subscribe()
    }

    pub fn versioned_snapshot(&self) -> VersionedCoreSnapshot {
        self.snapshot_tx.borrow().clone()
    }

    pub fn selected_processes_running(&self) -> watch::Receiver<bool> {
        self.selected_running_tx.subscribe()
    }

    pub fn shortcut_settings(&self) -> watch::Receiver<ShortcutSettings> {
        self.shortcut_settings_tx.subscribe()
    }

    pub fn widget_profile(&self) -> watch::Receiver<WidgetProfile> {
        self.widget_profile_tx.subscribe()
    }

    pub fn shortcut_availability(&self) -> watch::Receiver<ShortcutAvailability> {
        self.shortcut_availability_tx.subscribe()
    }

    pub fn shortcut_availability_diagnostic(&self) -> String {
        self.shortcut_availability_tx.borrow().diagnostic()
    }

    pub(crate) fn publish_shortcut_availability(&self, availability: ShortcutAvailability) {
        self.shortcut_availability_tx.send_if_modified(|published| {
            if *published == availability {
                return false;
            }
            published.clone_from(&availability);
            true
        });
    }

    pub async fn snapshot(&self) -> CoreSnapshot {
        self.snapshot_at(Instant::now()).await
    }

    pub async fn snapshot_at(&self, now: Instant) -> CoreSnapshot {
        let metadata = self.metadata.lock().await;
        let snapshot = self.state.read().await.snapshot().clone();
        snapshot_with_runtime(
            snapshot,
            metadata.active_timing,
            metadata.telemetry,
            metadata.manual_stopwatch.snapshot_at(now),
            now,
        )
    }

    pub async fn toggle_manual_stopwatch_at(&self, now: Instant) -> CoreSnapshot {
        let _mutation = self.mutation.lock().await;
        let enabled = self.widget_profile.read().await.manual_stopwatch.enabled;
        let mut metadata = self.metadata.lock().await;
        let snapshot = self.state.read().await.snapshot().clone();
        if enabled && snapshot.active_game.is_some() {
            metadata.manual_stopwatch.toggle(now);
        }
        let snapshot = snapshot_with_runtime(
            snapshot,
            metadata.active_timing,
            metadata.telemetry,
            metadata.manual_stopwatch.snapshot_at(now),
            now,
        );
        self.publish_snapshot(snapshot)
    }

    pub async fn reset_manual_stopwatch_at(&self, now: Instant) -> CoreSnapshot {
        let _mutation = self.mutation.lock().await;
        let enabled = self.widget_profile.read().await.manual_stopwatch.enabled;
        let mut metadata = self.metadata.lock().await;
        let snapshot = self.state.read().await.snapshot().clone();
        if enabled && snapshot.active_game.is_some() {
            metadata.manual_stopwatch.reset();
        }
        let snapshot = snapshot_with_runtime(
            snapshot,
            metadata.active_timing,
            metadata.telemetry,
            metadata.manual_stopwatch.snapshot_at(now),
            now,
        );
        self.publish_snapshot(snapshot)
    }

    pub async fn toggle_overlay(&self) -> CoreSnapshot {
        let _mutation = self.mutation.lock().await;
        {
            self.state.write().await.toggle_overlay();
        }
        let snapshot = self.snapshot_at(Instant::now()).await;
        self.publish_snapshot(snapshot)
    }

    pub async fn set_overlay_interactive(&self, interactive: bool) -> CoreSnapshot {
        let _mutation = self.mutation.lock().await;
        {
            self.state
                .write()
                .await
                .set_overlay_interactive(interactive);
        }
        let snapshot = self.snapshot_at(Instant::now()).await;
        self.publish_snapshot(snapshot)
    }

    pub async fn apply_bridge_observation(&self, observation: WindowObservation) -> CoreSnapshot {
        self.apply_bridge_observation_at(observation, Instant::now())
            .await
    }

    pub async fn apply_bridge_observation_at(
        &self,
        observation: WindowObservation,
        reported_at: Instant,
    ) -> CoreSnapshot {
        let _mutation = self.mutation.lock().await;
        if !observation_has_valid_geometry(&observation) {
            return self.clear_game_locked().await;
        }

        let allowed = self.allowed_process(&observation).await;
        let mut metadata = self.metadata.lock().await;
        let mut state = self.state.write().await;
        if let Some(allowed) = allowed {
            reset_telemetry_for_changed_group(&mut metadata, &allowed);
            reset_stopwatch_for_changed_process(&mut metadata, allowed.instance);
            (metadata.active_timing, metadata.timing_floor) = reconcile_process_timing(
                metadata.active_process,
                metadata.timing_floor,
                allowed.instance,
                allowed.timing,
                reported_at,
            );
            state.observe_game(
                observation.into_game(classification_from_identity(&allowed.identity)),
            );
            metadata.bridge_last_report = Some(reported_at);
            metadata.active_identity = Some(allowed.identity);
            metadata.active_process = Some(allowed.instance);
        } else {
            state.clear_game();
            metadata.bridge_last_report = None;
            metadata.active_identity = None;
            metadata.active_process = None;
            metadata.active_timing = None;
            metadata.timing_floor = None;
            metadata.telemetry = None;
            metadata.telemetry_sampler.reset();
            metadata.manual_stopwatch.reset();
        }
        let snapshot = snapshot_with_runtime(
            state.snapshot().clone(),
            metadata.active_timing,
            metadata.telemetry,
            metadata.manual_stopwatch.snapshot_at(reported_at),
            reported_at,
        );
        self.publish_snapshot(snapshot)
    }

    pub async fn clear_game(&self) -> CoreSnapshot {
        let _mutation = self.mutation.lock().await;
        self.clear_game_locked().await
    }

    pub async fn apply_x11_observation(
        &self,
        observation: Option<WindowObservation>,
    ) -> CoreSnapshot {
        let _mutation = self.mutation.lock().await;
        let Some(observation) = observation else {
            return self.clear_game_locked().await;
        };
        if observation.app_id.as_deref() == Some(OVERLAY_APP_ID) {
            return self.snapshot().await;
        }
        if !observation_has_valid_geometry(&observation) {
            return self.clear_game_locked().await;
        }

        let allowed = self.allowed_process(&observation).await;
        let observed_at = Instant::now();
        let mut metadata = self.metadata.lock().await;
        let mut state = self.state.write().await;
        if let Some(allowed) = allowed {
            reset_telemetry_for_changed_group(&mut metadata, &allowed);
            reset_stopwatch_for_changed_process(&mut metadata, allowed.instance);
            (metadata.active_timing, metadata.timing_floor) = reconcile_process_timing(
                metadata.active_process,
                metadata.timing_floor,
                allowed.instance,
                allowed.timing,
                observed_at,
            );
            state.observe_game(
                observation.into_game(classification_from_identity(&allowed.identity)),
            );
            metadata.bridge_last_report = None;
            metadata.active_identity = Some(allowed.identity);
            metadata.active_process = Some(allowed.instance);
        } else {
            state.clear_game();
            metadata.bridge_last_report = None;
            metadata.active_identity = None;
            metadata.active_process = None;
            metadata.active_timing = None;
            metadata.timing_floor = None;
            metadata.telemetry = None;
            metadata.telemetry_sampler.reset();
            metadata.manual_stopwatch.reset();
        }
        let snapshot = snapshot_with_runtime(
            state.snapshot().clone(),
            metadata.active_timing,
            metadata.telemetry,
            metadata.manual_stopwatch.snapshot_at(observed_at),
            observed_at,
        );
        self.publish_snapshot(snapshot)
    }

    pub async fn expire_bridge_lease_at(&self, now: Instant) -> bool {
        let _mutation = self.mutation.lock().await;
        let mut metadata = self.metadata.lock().await;
        let Some(reported_at) = metadata.bridge_last_report else {
            return false;
        };
        if now.checked_duration_since(reported_at).unwrap_or_default() < BRIDGE_LEASE_TIMEOUT {
            return false;
        }

        let mut state = self.state.write().await;
        state.clear_game();
        metadata.bridge_last_report = None;
        metadata.active_identity = None;
        metadata.active_process = None;
        metadata.active_timing = None;
        metadata.timing_floor = None;
        metadata.telemetry = None;
        metadata.telemetry_sampler.reset();
        metadata.manual_stopwatch.reset();
        self.publish_snapshot(state.snapshot().clone());
        true
    }

    pub async fn install_process_snapshot(&self, new_processes: HashMap<u32, ProcessInfo>) {
        self.install_process_snapshot_at(new_processes, Instant::now())
            .await;
    }

    pub async fn install_process_snapshot_at(
        &self,
        new_processes: HashMap<u32, ProcessInfo>,
        observed_at: Instant,
    ) {
        self.install_refresh_snapshot_at(
            new_processes,
            TemperatureSnapshot::default(),
            observed_at,
        )
        .await;
    }

    pub(crate) async fn install_refresh_snapshot_at(
        &self,
        new_processes: HashMap<u32, ProcessInfo>,
        temperatures: TemperatureSnapshot,
        observed_at: Instant,
    ) {
        let _mutation = self.mutation.lock().await;
        {
            let mut processes = self.processes.write().await;
            *processes = new_processes;
        }

        let settings = self.settings.read().await.clone();
        let allowlist = GameAllowlist::from_settings(&settings);
        let processes = self.processes.read().await;
        self.publish_selected_running(snapshot_has_selected_process(&allowlist, &processes));

        let mut metadata = self.metadata.lock().await;
        let mut state = self.state.write().await;
        let is_valid = state.snapshot().active_game.as_ref().is_none_or(|game| {
            active_game_is_current(
                game,
                metadata.active_identity.as_ref(),
                metadata.active_process.as_ref(),
                &processes,
                &allowlist,
            )
        });
        if !is_valid {
            state.clear_game();
            metadata.bridge_last_report = None;
            metadata.active_identity = None;
            metadata.active_process = None;
            metadata.active_timing = None;
            metadata.timing_floor = None;
            metadata.telemetry = None;
            metadata.telemetry_sampler.reset();
            metadata.manual_stopwatch.reset();
        } else {
            refresh_active_process(
                &mut metadata,
                state.snapshot().active_game.as_ref(),
                &processes,
                observed_at,
            );
            refresh_telemetry(
                &mut metadata,
                state.snapshot().active_game.as_ref(),
                &processes,
                temperatures,
                observed_at,
            );
        }
        let snapshot = snapshot_with_runtime(
            state.snapshot().clone(),
            metadata.active_timing,
            metadata.telemetry,
            metadata.manual_stopwatch.snapshot_at(observed_at),
            observed_at,
        );
        self.publish_snapshot(snapshot);
    }

    pub async fn reload_settings(
        &self,
        settings: LifecycleSettings,
    ) -> Result<CoreSnapshot, RuntimeSettingsError> {
        let settings = validate_enabled_settings(settings)?;
        let shortcut_settings = settings.shortcut.clone();
        let allowlist = GameAllowlist::from_settings(&settings);
        let _mutation = self.mutation.lock().await;

        {
            let mut current = self.settings.write().await;
            *current = settings;
        }
        self.publish_shortcut_settings(shortcut_settings);

        let processes = self.processes.read().await;
        self.publish_selected_running(snapshot_has_selected_process(&allowlist, &processes));

        let mut metadata = self.metadata.lock().await;
        let mut state = self.state.write().await;
        let is_valid = state.snapshot().active_game.as_ref().is_none_or(|game| {
            active_game_is_current(
                game,
                metadata.active_identity.as_ref(),
                metadata.active_process.as_ref(),
                &processes,
                &allowlist,
            )
        });
        if !is_valid {
            state.clear_game();
            metadata.bridge_last_report = None;
            metadata.active_identity = None;
            metadata.active_process = None;
            metadata.active_timing = None;
            metadata.timing_floor = None;
            metadata.telemetry = None;
            metadata.telemetry_sampler.reset();
            metadata.manual_stopwatch.reset();
        } else {
            refresh_active_process(
                &mut metadata,
                state.snapshot().active_game.as_ref(),
                &processes,
                Instant::now(),
            );
        }
        let now = Instant::now();
        let snapshot = snapshot_with_runtime(
            state.snapshot().clone(),
            metadata.active_timing,
            metadata.telemetry,
            metadata.manual_stopwatch.snapshot_at(now),
            now,
        );
        Ok(self.publish_snapshot(snapshot))
    }

    pub async fn reload_widget_profile(
        &self,
        profile: WidgetProfile,
    ) -> Result<CoreSnapshot, WidgetProfileError> {
        let profile = profile.validate()?;
        let _mutation = self.mutation.lock().await;
        {
            let mut current = self.widget_profile.write().await;
            current.clone_from(&profile);
        }
        self.publish_widget_profile(profile);

        let now = Instant::now();
        let metadata = self.metadata.lock().await;
        let snapshot = self.state.read().await.snapshot().clone();
        let snapshot = snapshot_with_runtime(
            snapshot,
            metadata.active_timing,
            metadata.telemetry,
            metadata.manual_stopwatch.snapshot_at(now),
            now,
        );
        Ok(self.publish_snapshot(snapshot))
    }

    async fn clear_game_locked(&self) -> CoreSnapshot {
        let mut metadata = self.metadata.lock().await;
        let mut state = self.state.write().await;
        state.clear_game();
        metadata.bridge_last_report = None;
        metadata.active_identity = None;
        metadata.active_process = None;
        metadata.active_timing = None;
        metadata.timing_floor = None;
        metadata.telemetry = None;
        metadata.telemetry_sampler.reset();
        metadata.manual_stopwatch.reset();
        self.publish_snapshot(state.snapshot().clone())
    }

    async fn allowed_process(&self, observation: &WindowObservation) -> Option<AllowedProcess> {
        let pid = observation.pid?;
        let settings = self.settings.read().await.clone();
        let allowlist = GameAllowlist::from_settings(&settings);
        let processes = self.processes.read().await;
        let process = processes.get(&pid)?;
        let identity = classify_process_identity(pid, &processes);
        allowlist
            .allows_identity(&identity)
            .then_some(AllowedProcess {
                identity,
                instance: ProcessInstance {
                    pid,
                    start_ticks: process.start_ticks,
                },
                timing: process.timing,
            })
    }

    fn publish_snapshot(&self, snapshot: CoreSnapshot) -> CoreSnapshot {
        self.snapshot_tx.send_if_modified(|published| {
            if !snapshot_state_changed(&published.snapshot, &snapshot) {
                return false;
            }
            published.revision = published.revision.saturating_add(1);
            published.snapshot.clone_from(&snapshot);
            true
        });
        snapshot
    }

    fn publish_selected_running(&self, selected_running: bool) {
        if *self.selected_running_tx.borrow() != selected_running {
            self.selected_running_tx.send_replace(selected_running);
        }
    }

    fn publish_shortcut_settings(&self, settings: ShortcutSettings) {
        self.shortcut_settings_tx.send_if_modified(|published| {
            if *published == settings {
                return false;
            }
            published.clone_from(&settings);
            true
        });
    }

    fn publish_widget_profile(&self, profile: WidgetProfile) {
        self.widget_profile_tx.send_if_modified(|published| {
            if *published == profile {
                return false;
            }
            published.clone_from(&profile);
            true
        });
    }
}

pub async fn run_process_refresh(runtime: CoreRuntime) {
    let mut refresh = tokio::time::interval(PROCESS_REFRESH_INTERVAL);
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    refresh.tick().await;

    loop {
        refresh.tick().await;
        match tokio::task::spawn_blocking(|| {
            let processes = scan_processes();
            let temperatures =
                scan_temperatures(Path::new("/sys/class/hwmon"), Path::new("/sys/devices"));
            (processes, temperatures)
        })
        .await
        {
            Ok((Ok(processes), temperatures)) => {
                runtime
                    .install_refresh_snapshot_at(processes, temperatures, Instant::now())
                    .await;
            }
            Ok((Err(error), temperatures)) => {
                runtime
                    .install_refresh_snapshot_at(HashMap::new(), temperatures, Instant::now())
                    .await;
                eprintln!("OverCrow procfs refresh failed: {error:#}");
            }
            Err(error) => {
                runtime.install_process_snapshot(HashMap::new()).await;
                eprintln!("OverCrow procfs refresh task failed: {error}");
            }
        }
    }
}

pub async fn run_bridge_watchdog(runtime: CoreRuntime) {
    let mut watchdog = tokio::time::interval(BRIDGE_WATCHDOG_INTERVAL);
    watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        watchdog.tick().await;
        runtime.expire_bridge_lease_at(Instant::now()).await;
    }
}

fn validate_enabled_settings(
    settings: LifecycleSettings,
) -> Result<LifecycleSettings, RuntimeSettingsError> {
    let settings = settings.validate().map_err(RuntimeSettingsError::Invalid)?;
    if !settings.enabled {
        return Err(RuntimeSettingsError::Disabled);
    }
    Ok(settings)
}

fn observation_has_valid_geometry(observation: &WindowObservation) -> bool {
    observation.scale.is_finite()
        && observation.scale > 0.0
        && observation.rect.width > 0
        && observation.rect.height > 0
}

fn classification_from_identity(identity: &ProcessIdentity) -> ProcessClassification {
    ProcessClassification {
        steam_app_id: identity.steam_app_id,
        is_game_candidate: identity.game_candidate,
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

fn matching_process_identity(
    game: &GameWindow,
    processes: &HashMap<u32, ProcessInfo>,
    allowlist: &GameAllowlist,
) -> Option<ProcessIdentity> {
    let pid = game.pid?;
    let identity = classify_process_identity(pid, processes);
    (allowlist.allows_identity(&identity) && game.steam_app_id == identity.steam_app_id)
        .then_some(identity)
}

fn active_game_process(
    game: &GameWindow,
    processes: &HashMap<u32, ProcessInfo>,
) -> Option<(ProcessInstance, Option<ProcessTiming>)> {
    let pid = game.pid?;
    let process = processes.get(&pid)?;
    Some((
        ProcessInstance {
            pid,
            start_ticks: process.start_ticks,
        },
        process.timing,
    ))
}

fn refresh_active_process(
    metadata: &mut RuntimeMetadata,
    game: Option<&GameWindow>,
    processes: &HashMap<u32, ProcessInfo>,
    observed_at: Instant,
) {
    let Some((instance, timing)) = game.and_then(|game| active_game_process(game, processes))
    else {
        metadata.active_process = None;
        metadata.active_timing = None;
        metadata.timing_floor = None;
        metadata.manual_stopwatch.reset();
        return;
    };
    reset_stopwatch_for_changed_process(metadata, instance);
    (metadata.active_timing, metadata.timing_floor) = reconcile_process_timing(
        metadata.active_process,
        metadata.timing_floor,
        instance,
        timing,
        observed_at,
    );
    metadata.active_process = Some(instance);
}

fn reset_telemetry_for_changed_group(metadata: &mut RuntimeMetadata, allowed: &AllowedProcess) {
    let current_steam_app_id = metadata
        .active_identity
        .as_ref()
        .and_then(|identity| identity.steam_app_id);
    if metadata.active_process != Some(allowed.instance)
        || current_steam_app_id != allowed.identity.steam_app_id
    {
        metadata.telemetry = None;
        metadata.telemetry_sampler.reset();
    }
}

fn reset_stopwatch_for_changed_process(
    metadata: &mut RuntimeMetadata,
    current_instance: ProcessInstance,
) {
    if metadata.active_process != Some(current_instance) {
        metadata.manual_stopwatch.reset();
    }
}

fn refresh_telemetry(
    metadata: &mut RuntimeMetadata,
    game: Option<&GameWindow>,
    processes: &HashMap<u32, ProcessInfo>,
    temperatures: TemperatureSnapshot,
    observed_at: Instant,
) {
    let Some(game) = game else {
        metadata.telemetry = None;
        metadata.telemetry_sampler.reset();
        return;
    };
    let process_telemetry = game
        .pid
        .and_then(|pid| collect_process_sample(pid, game.steam_app_id, processes, observed_at))
        .map(|sample| {
            metadata
                .telemetry_sampler
                .observe(sample, procfs::ticks_per_second())
        })
        .unwrap_or_else(|| {
            metadata.telemetry_sampler.reset();
            GameTelemetry::default()
        });
    metadata.telemetry = Some(GameTelemetry {
        cpu_temperature_millicelsius: temperatures.cpu_millicelsius,
        gpu_temperature_millicelsius: temperatures.gpu_millicelsius,
        ..process_telemetry
    });
}

fn reconcile_process_timing(
    previous_instance: Option<ProcessInstance>,
    previous_floor: Option<ProcessTiming>,
    current_instance: ProcessInstance,
    current_timing: Option<ProcessTiming>,
    observed_at: Instant,
) -> (Option<ProcessTiming>, Option<ProcessTiming>) {
    if previous_instance != Some(current_instance) {
        return (current_timing, current_timing);
    }
    let Some(current) = current_timing else {
        return (None, previous_floor);
    };
    let current = match previous_floor {
        Some(previous) => {
            let floor = previous.elapsed_at(observed_at);
            if floor > current.elapsed_at(observed_at) {
                ProcessTiming::new(floor, observed_at)
            } else {
                current
            }
        }
        None => current,
    };
    (Some(current), Some(current))
}

fn snapshot_with_runtime(
    mut snapshot: CoreSnapshot,
    timing: Option<ProcessTiming>,
    telemetry: Option<GameTelemetry>,
    manual_stopwatch: ManualStopwatchSnapshot,
    now: Instant,
) -> CoreSnapshot {
    snapshot.session_elapsed_ms = snapshot.active_game.as_ref().and_then(|_| {
        timing.map(|timing| u64::try_from(timing.elapsed_at(now).as_millis()).unwrap_or(u64::MAX))
    });
    snapshot.telemetry = snapshot.active_game.as_ref().and(telemetry);
    snapshot.manual_stopwatch = manual_stopwatch;
    snapshot
}

fn active_game_is_current(
    game: &GameWindow,
    accepted_identity: Option<&ProcessIdentity>,
    accepted_process: Option<&ProcessInstance>,
    processes: &HashMap<u32, ProcessInfo>,
    allowlist: &GameAllowlist,
) -> bool {
    let current_identity = matching_process_identity(game, processes, allowlist);
    let current_process = active_game_process(game, processes).map(|(instance, _)| instance);
    current_identity
        .as_ref()
        .is_some_and(|identity| Some(identity) == accepted_identity)
        && current_process
            .as_ref()
            .is_some_and(|process| Some(process) == accepted_process)
}

fn snapshot_state_changed(current: &CoreSnapshot, next: &CoreSnapshot) -> bool {
    let CoreSnapshot {
        active_game: current_game,
        overlay_mode: current_mode,
        session_elapsed_ms: _,
        telemetry: current_telemetry,
        manual_stopwatch: current_stopwatch,
    } = current;
    let CoreSnapshot {
        active_game: next_game,
        overlay_mode: next_mode,
        session_elapsed_ms: _,
        telemetry: next_telemetry,
        manual_stopwatch: next_stopwatch,
    } = next;
    current_game != next_game
        || current_mode != next_mode
        || current_telemetry != next_telemetry
        || current_stopwatch.running != next_stopwatch.running
        || (!next_stopwatch.running && current_stopwatch.elapsed_ms != next_stopwatch.elapsed_ms)
}
