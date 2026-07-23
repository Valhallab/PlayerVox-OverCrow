use std::time::{Duration, Instant as StdInstant};

use anyhow::{Context, bail};
use overcrow_logging::EventLogger;
use overcrow_protocol::{Core1Proxy, CoreSnapshot, OverlayMode};
use serde_json::Value;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt},
    time::{Instant, MissedTickBehavior, interval, sleep},
};

use crate::{
    geometry::GeometrySynchronizer,
    ipc::{HyprlandIpc, MAX_EVENT_LINE_BYTES, UnsupportedShortcutBackend},
    model::{HyprMonitor, HyprWindow, OVERLAY_APP_ID, WindowAddress, WindowReport},
    reconcile::Reconciler,
    shortcut::{
        GlobalShortcut, HyprBinding, ShortcutBackend, ShortcutDecision, ShortcutReconciler,
        ShortcutSpec,
    },
};

pub const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(2);
pub const FOCUS_SYNC_INTERVAL: Duration = Duration::from_millis(250);
pub const GEOMETRY_SYNC_INTERVAL: Duration = Duration::from_millis(33);
pub const EVENT_DEBOUNCE: Duration = Duration::from_millis(25);
pub const INTERACTIVE_TAG: &str = "overcrow-interactive";
pub const GAME_INPUT_BLOCKED_TAG: &str = "overcrow-game-input-blocked";
const DEBOUNCE_IDLE_DURATION: Duration = Duration::from_secs(24 * 60 * 60);
const DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
struct DiagnosticRateLimiter {
    interval: Duration,
    next: Option<StdInstant>,
    suppressed: u64,
}

impl DiagnosticRateLimiter {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            next: None,
            suppressed: 0,
        }
    }

    fn admit_at(&mut self, now: StdInstant) -> Option<u64> {
        if self.next.is_none_or(|deadline| now >= deadline) {
            let suppressed = std::mem::take(&mut self.suppressed);
            self.next = now.checked_add(self.interval);
            return Some(suppressed);
        }
        self.suppressed = self.suppressed.saturating_add(1);
        None
    }
}

struct BridgeDiagnostics {
    logger: EventLogger,
    focus: DiagnosticRateLimiter,
    geometry: DiagnosticRateLimiter,
}

impl BridgeDiagnostics {
    fn new(logger: EventLogger) -> Self {
        Self {
            logger,
            focus: DiagnosticRateLimiter::new(DIAGNOSTIC_INTERVAL),
            geometry: DiagnosticRateLimiter::new(DIAGNOSTIC_INTERVAL),
        }
    }

    #[cfg(test)]
    fn disabled() -> Self {
        Self::new(EventLogger::disabled())
    }

    fn focus_batch(&mut self, command_count: usize, clear_core: bool) {
        if command_count == 0 {
            return;
        }
        if let Some(suppressed) = self.focus.admit_at(StdInstant::now()) {
            self.logger.info(
                "focus_command_batch",
                format_args!(
                    "commands={command_count} clear_core={clear_core} suppressed={suppressed}"
                ),
            );
        }
    }

    fn geometry_batch(&mut self, command_count: usize) {
        if command_count == 0 {
            return;
        }
        if let Some(suppressed) = self.geometry.admit_at(StdInstant::now()) {
            self.logger.info(
                "geometry_command_batch",
                format_args!("commands={command_count} suppressed={suppressed}"),
            );
        }
    }
}

trait DesktopIpc {
    async fn active_window(&self) -> anyhow::Result<Option<HyprWindow>>;
    async fn clients(&self) -> anyhow::Result<Vec<HyprWindow>>;
    async fn monitors(&self) -> anyhow::Result<Vec<HyprMonitor>>;
    async fn global_shortcuts(&self) -> anyhow::Result<Vec<GlobalShortcut>>;
    async fn bindings(&self) -> anyhow::Result<Vec<HyprBinding>>;
    async fn shortcut_backend(&self) -> anyhow::Result<ShortcutBackend>;
    async fn dispatch(&self, command: &str) -> anyhow::Result<()>;
}

impl DesktopIpc for HyprlandIpc {
    async fn active_window(&self) -> anyhow::Result<Option<HyprWindow>> {
        decode_active_window(self.query("activewindow").await?)
    }

    async fn clients(&self) -> anyhow::Result<Vec<HyprWindow>> {
        self.query("clients").await
    }

    async fn monitors(&self) -> anyhow::Result<Vec<HyprMonitor>> {
        self.query("monitors").await
    }

    async fn global_shortcuts(&self) -> anyhow::Result<Vec<GlobalShortcut>> {
        self.query("globalshortcuts").await
    }

    async fn bindings(&self) -> anyhow::Result<Vec<HyprBinding>> {
        HyprlandIpc::bindings(self).await
    }

    async fn shortcut_backend(&self) -> anyhow::Result<ShortcutBackend> {
        HyprlandIpc::shortcut_backend(self).await
    }

    async fn dispatch(&self, command: &str) -> anyhow::Result<()> {
        HyprlandIpc::dispatch(self, command).await
    }
}

trait CoreReporter {
    async fn report(&self, report: &WindowReport) -> anyhow::Result<CoreSnapshot>;
    async fn clear(&self) -> anyhow::Result<CoreSnapshot>;
    async fn snapshot(&self) -> anyhow::Result<CoreSnapshot>;
}

struct DbusCoreReporter<'a> {
    proxy: Core1Proxy<'a>,
}

impl CoreReporter for DbusCoreReporter<'_> {
    async fn report(&self, report: &WindowReport) -> anyhow::Result<CoreSnapshot> {
        let json = self
            .proxy
            .report_window(
                report.pid,
                &report.title,
                &report.app_id,
                report.rect.x,
                report.rect.y,
                i32::try_from(report.rect.width).context("overlay width exceeds i32")?,
                i32::try_from(report.rect.height).context("overlay height exceeds i32")?,
                &report.scale.to_string(),
            )
            .await
            .context("Core1 ReportWindow failed")?;
        decode_core_snapshot(&json)
    }

    async fn clear(&self) -> anyhow::Result<CoreSnapshot> {
        let json = self
            .proxy
            .clear_window()
            .await
            .context("Core1 ClearWindow failed")?;
        decode_core_snapshot(&json)
    }

    async fn snapshot(&self) -> anyhow::Result<CoreSnapshot> {
        let json = self
            .proxy
            .snapshot()
            .await
            .context("Core1 Snapshot failed")?;
        decode_core_snapshot(&json)
    }
}

fn decode_core_snapshot(json: &str) -> anyhow::Result<CoreSnapshot> {
    serde_json::from_str(json).context("Core1 returned a malformed snapshot")
}

#[derive(Debug, Default)]
struct FocusSyncState {
    observed_mode: Option<OverlayMode>,
    interactive_focus_pending: bool,
    passive_focus_pending: bool,
}

impl FocusSyncState {
    fn observe_mode(&mut self, mode: OverlayMode) {
        if self.observed_mode == Some(mode) {
            return;
        }

        match mode {
            OverlayMode::Interactive => {
                self.interactive_focus_pending = true;
                self.passive_focus_pending = false;
            }
            OverlayMode::Passive => {
                self.interactive_focus_pending = false;
                self.passive_focus_pending = self.observed_mode == Some(OverlayMode::Interactive);
            }
        }
        self.observed_mode = Some(mode);
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct FocusPlan {
    commands: Vec<String>,
    clear_core: bool,
}

#[derive(Debug)]
struct InteractionTargets {
    game: WindowAddress,
    overlay: WindowAddress,
}

fn reconcile_window_tag(
    clients: &[HyprWindow],
    tag: &str,
    intended: Option<&WindowAddress>,
) -> (Vec<String>, bool) {
    let mut commands = Vec::new();
    for window in clients {
        let Some(address) = WindowAddress::parse(&window.address) else {
            continue;
        };
        if window.tags.iter().any(|candidate| candidate == tag)
            && intended.is_none_or(|target| target != &address)
        {
            commands.push(format!(
                "dispatch tagwindow -{tag} address:{}",
                address.as_str()
            ));
        }
    }

    let target_has_tag = intended.is_some_and(|address| {
        clients
            .iter()
            .find(|window| window.address == address.as_str())
            .is_some_and(|window| window.tags.iter().any(|candidate| candidate == tag))
    });
    if let Some(address) = intended
        && !target_has_tag
    {
        commands.push(format!(
            "dispatch tagwindow +{tag} address:{}",
            address.as_str()
        ));
    }

    (commands, target_has_tag)
}

fn focus_plan(
    clients: &[HyprWindow],
    active: Option<&HyprWindow>,
    snapshot: &CoreSnapshot,
    state: &mut FocusSyncState,
) -> FocusPlan {
    let targets = (snapshot.overlay_mode == OverlayMode::Interactive)
        .then(|| interaction_targets(clients, snapshot))
        .flatten();
    let effective_mode = if targets.is_some() {
        OverlayMode::Interactive
    } else {
        OverlayMode::Passive
    };
    state.observe_mode(effective_mode);

    let intended_game = targets.as_ref().map(|targets| &targets.game);
    let (mut commands, game_tagged) =
        reconcile_window_tag(clients, GAME_INPUT_BLOCKED_TAG, intended_game);

    let intended_overlay = targets.as_ref().map(|targets| &targets.overlay);
    let (overlay_commands, overlay_tagged) =
        reconcile_window_tag(clients, INTERACTIVE_TAG, intended_overlay);
    commands.extend(overlay_commands);

    match effective_mode {
        OverlayMode::Interactive => {
            if let Some(targets) = &targets
                && game_tagged
                && overlay_tagged
                && commands.is_empty()
            {
                // Global shortcuts (e.g. manual stopwatch) can steal focus while the
                // game remains input-blocked. Recover when nothing usable is focused
                // (no window, or the blocked game). Do not steal from other apps.
                let overlay_focused =
                    active.is_some_and(|window| window.address == targets.overlay.as_str());
                let game_focused =
                    active.is_some_and(|window| window.address == targets.game.as_str());
                let can_take_focus = active.is_none() || game_focused;
                let should_focus = !overlay_focused
                    && can_take_focus
                    && (state.interactive_focus_pending || game_focused);
                if should_focus {
                    commands.push(format!(
                        "dispatch focuswindow address:{}",
                        targets.overlay.as_str()
                    ));
                    state.interactive_focus_pending = false;
                }
            }
        }
        OverlayMode::Passive => {
            let managed_tags_present = clients.iter().any(|window| {
                window
                    .tags
                    .iter()
                    .any(|tag| matches!(tag.as_str(), INTERACTIVE_TAG | GAME_INPUT_BLOCKED_TAG))
            });
            if snapshot.overlay_mode == OverlayMode::Passive
                && state.passive_focus_pending
                && !managed_tags_present
            {
                if let Some(address) = validated_game_address(clients, snapshot) {
                    commands.push(format!("dispatch focuswindow address:{}", address.as_str()));
                }
                state.passive_focus_pending = false;
            }
        }
    }

    FocusPlan {
        commands,
        clear_core: snapshot.overlay_mode == OverlayMode::Interactive && targets.is_none(),
    }
}

#[cfg(test)]
fn focus_commands(
    clients: &[HyprWindow],
    snapshot: &CoreSnapshot,
    state: &mut FocusSyncState,
) -> Vec<String> {
    // Tests that omit active-window context behave like "no usable focus".
    focus_plan(clients, None, snapshot, state).commands
}

#[cfg(test)]
fn focus_commands_with_active(
    clients: &[HyprWindow],
    active: Option<&HyprWindow>,
    snapshot: &CoreSnapshot,
    state: &mut FocusSyncState,
) -> Vec<String> {
    focus_plan(clients, active, snapshot, state).commands
}

fn unique_overlay_address(clients: &[HyprWindow]) -> Option<WindowAddress> {
    let mut overlays = clients
        .iter()
        .filter(|window| window.mapped && !window.hidden && window.class == OVERLAY_APP_ID);
    let overlay = overlays.next()?;
    if overlays.next().is_some() {
        return None;
    }
    WindowAddress::parse(&overlay.address)
}

fn interaction_targets(
    clients: &[HyprWindow],
    snapshot: &CoreSnapshot,
) -> Option<InteractionTargets> {
    Some(InteractionTargets {
        game: validated_game_address(clients, snapshot)?,
        overlay: unique_overlay_address(clients)?,
    })
}

fn cleanup_commands(clients: &[HyprWindow]) -> Vec<String> {
    let mut commands = Vec::new();
    for window in clients {
        let Some(address) = WindowAddress::parse(&window.address) else {
            continue;
        };
        for tag in [GAME_INPUT_BLOCKED_TAG, INTERACTIVE_TAG] {
            if window.tags.iter().any(|candidate| candidate == tag) {
                commands.push(format!(
                    "dispatch tagwindow -{tag} address:{}",
                    address.as_str()
                ));
            }
        }
    }
    commands
}

async fn cleanup_focus_state_with<D: DesktopIpc>(desktop: &D) -> anyhow::Result<()> {
    let clients = desktop.clients().await?;
    for command in cleanup_commands(&clients) {
        desktop.dispatch(&command).await?;
    }
    Ok(())
}

pub async fn cleanup_focus_state(ipc: &HyprlandIpc) -> anyhow::Result<()> {
    cleanup_focus_state_with(ipc).await
}

async fn cleanup_shortcut_state_with<D: DesktopIpc>(desktop: &D) -> anyhow::Result<()> {
    let backend = desktop.shortcut_backend().await?;
    let bindings = desktop.bindings().await?;
    let mut processed = std::collections::BTreeSet::new();

    for binding in &bindings {
        let Some(spec) = ShortcutSpec::from_owned_binding(binding) else {
            continue;
        };
        let identity = (spec.modmask(), spec.key().to_owned());
        if !processed.insert(identity) {
            continue;
        }
        let matching = bindings
            .iter()
            .filter(|candidate| spec.matches_accelerator(candidate))
            .collect::<Vec<_>>();
        if matching.len() == 1 && spec.owns(matching[0]) {
            desktop.dispatch(&spec.unbind_request(backend)).await?;
        }
    }
    Ok(())
}

async fn cleanup_runtime_state_with<D: DesktopIpc>(desktop: &D) -> anyhow::Result<()> {
    let (focus, shortcut) = tokio::join!(
        cleanup_focus_state_with(desktop),
        cleanup_shortcut_state_with(desktop)
    );
    match (focus, shortcut) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error.context("focus-state cleanup failed")),
        (Ok(()), Err(error)) => Err(error.context("shortcut cleanup failed")),
        (Err(focus), Err(shortcut)) => Err(anyhow::anyhow!(
            "focus-state cleanup failed: {focus:#}; shortcut cleanup failed: {shortcut:#}"
        )),
    }
}

pub async fn cleanup_runtime_state(ipc: &HyprlandIpc) -> anyhow::Result<()> {
    cleanup_runtime_state_with(ipc).await
}

fn validated_game_address(
    clients: &[HyprWindow],
    snapshot: &CoreSnapshot,
) -> Option<WindowAddress> {
    let game = snapshot.active_game.as_ref()?;
    let pid = i64::from(game.pid?);
    let app_id = game.app_id.as_deref()?;
    let mut matching = clients.iter().filter(|window| {
        window.mapped
            && !window.hidden
            && window.pid == pid
            && window.class == app_id
            && window.class != OVERLAY_APP_ID
    });
    let window = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    WindowAddress::parse(&window.address)
}

async fn sync_focus_once<D: DesktopIpc, C: CoreReporter>(
    desktop: &D,
    core: &C,
    state: &mut FocusSyncState,
    diagnostics: &mut BridgeDiagnostics,
) -> anyhow::Result<CoreSnapshot> {
    let (clients, active) = tokio::try_join!(desktop.clients(), desktop.active_window())?;
    let snapshot = core.snapshot().await;
    let passive_fallback = CoreSnapshot::default();
    let desired = snapshot.as_ref().unwrap_or(&passive_fallback);
    let plan = focus_plan(&clients, active.as_ref(), desired, state);
    let command_count = plan.commands.len();
    for command in plan.commands {
        desktop.dispatch(&command).await?;
    }
    diagnostics.focus_batch(command_count, plan.clear_core);
    if plan.clear_core {
        return core.clear().await;
    }
    snapshot
}

async fn sync_geometry_once<D: DesktopIpc>(
    desktop: &D,
    snapshot: &CoreSnapshot,
    synchronizer: &mut GeometrySynchronizer,
    diagnostics: &mut BridgeDiagnostics,
) -> anyhow::Result<()> {
    if snapshot.active_game.is_none() {
        return Ok(());
    }
    let (active, clients) = tokio::try_join!(desktop.active_window(), desktop.clients())?;
    let commands = synchronizer.commands(snapshot, active.as_ref(), &clients);
    let command_count = commands.len();
    for command in commands {
        desktop.dispatch(&command).await?;
    }
    diagnostics.geometry_batch(command_count);
    Ok(())
}

#[cfg(test)]
async fn sync_shortcut_once<D: DesktopIpc>(
    desktop: &D,
    snapshot: &CoreSnapshot,
    reconciler: &mut ShortcutReconciler,
) -> anyhow::Result<()> {
    sync_shortcuts_once(desktop, snapshot, std::slice::from_mut(reconciler)).await
}

async fn sync_shortcuts_once<D: DesktopIpc>(
    desktop: &D,
    snapshot: &CoreSnapshot,
    reconcilers: &mut [ShortcutReconciler],
) -> anyhow::Result<()> {
    let desired = snapshot.active_game.is_some();
    let probes = reconcilers
        .iter_mut()
        .map(|reconciler| reconciler.needs_probe(desired))
        .collect::<Vec<_>>();
    if probes.iter().all(|probe| !probe) {
        return Ok(());
    }

    let (globals, bindings) = tokio::try_join!(desktop.global_shortcuts(), desktop.bindings())?;
    for (reconciler, probe) in reconcilers.iter_mut().zip(probes) {
        if !probe {
            continue;
        }
        match reconciler.plan(desired, &globals, &bindings) {
            ShortcutDecision::Bind { request } => {
                desktop.dispatch(&request).await?;
                reconciler.mark_bound();
            }
            ShortcutDecision::Unbind { request } => {
                desktop.dispatch(&request).await?;
                reconciler.mark_unbound();
            }
            ShortcutDecision::Conflict => {
                if reconciler.take_warning() {
                    eprintln!(
                        "OverCrow shortcut is unavailable because {} is already bound",
                        reconciler.spec().keys()
                    );
                }
            }
            ShortcutDecision::AmbiguousOwnership => {
                if reconciler.take_warning() {
                    eprintln!(
                        "OverCrow shortcut cleanup skipped because ownership of {} is ambiguous",
                        reconciler.spec().keys()
                    );
                }
            }
            ShortcutDecision::Idle | ShortcutDecision::WaitForPortal => {}
        }
    }
    Ok(())
}

#[cfg(test)]
fn prepare_shortcut_reconciler(
    spec: ShortcutSpec,
    backend: anyhow::Result<ShortcutBackend>,
) -> anyhow::Result<(Option<ShortcutReconciler>, Option<String>)> {
    match backend {
        Ok(backend) => Ok((Some(ShortcutReconciler::new(spec, backend)), None)),
        Err(error) if error.downcast_ref::<UnsupportedShortcutBackend>().is_some() => Ok((
            None,
            Some(format!(
                "OverCrow shortcut integration is disabled: {error}"
            )),
        )),
        Err(error) => Err(error),
    }
}

fn prepare_shortcut_reconcilers(
    overlay: Option<ShortcutSpec>,
    backend: anyhow::Result<ShortcutBackend>,
) -> anyhow::Result<(Vec<ShortcutReconciler>, Option<String>)> {
    match backend {
        Ok(backend) => {
            let mut specs = Vec::with_capacity(3);
            if let Some(overlay) = overlay {
                specs.push(overlay);
            }
            specs.extend(ShortcutSpec::manual_stopwatch()?);
            Ok((
                specs
                    .into_iter()
                    .map(|spec| ShortcutReconciler::new(spec, backend))
                    .collect(),
                None,
            ))
        }
        Err(error) if error.downcast_ref::<UnsupportedShortcutBackend>().is_some() => Ok((
            Vec::new(),
            Some(format!(
                "OverCrow shortcut integration is disabled: {error}"
            )),
        )),
        Err(error) => Err(error),
    }
}

fn decode_active_window(value: Value) -> anyhow::Result<Option<HyprWindow>> {
    if value.as_object().is_some_and(serde_json::Map::is_empty) {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .context("Hyprland returned a malformed active window")
}

async fn reconcile_once<D: DesktopIpc, C: CoreReporter>(
    desktop: &D,
    core: &C,
    reconciler: &mut Reconciler,
) -> anyhow::Result<CoreSnapshot> {
    let result = async {
        let (active, clients, monitors, current) = tokio::try_join!(
            desktop.active_window(),
            desktop.clients(),
            desktop.monitors(),
            core.snapshot(),
        )?;
        let preserve_game = current.overlay_mode == OverlayMode::Interactive;
        let output = reconciler.reconcile(active.as_ref(), &clients, &monitors, preserve_game);
        let snapshot = match output.report.as_ref() {
            Some(report) => core.report(report).await?,
            None => core.clear().await?,
        };
        Ok(snapshot)
    }
    .await;

    if result.is_err() {
        let _ = core.clear().await;
    }
    result
}

pub fn is_reconciliation_event(line: &str) -> bool {
    let Some((name, _)) = line.split_once(">>") else {
        return false;
    };
    matches!(
        name,
        "workspace"
            | "workspacev2"
            | "focusedmon"
            | "focusedmonv2"
            | "activewindow"
            | "activewindowv2"
            | "fullscreen"
            | "monitorremoved"
            | "monitorremovedv2"
            | "monitoradded"
            | "monitoraddedv2"
            | "openwindow"
            | "closewindow"
            | "kill"
            | "movewindow"
            | "movewindowv2"
            | "changefloatingmode"
            | "windowtitle"
            | "windowtitlev2"
            | "configreloaded"
            | "pin"
            | "minimized"
    )
}

async fn read_event_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> anyhow::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .context("failed to read Hyprland event")?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            bail!("Hyprland event socket ended with a partial line");
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |position| position + 1);
        let data_bytes = newline.map_or(consumed, |position| position);
        if line.len() + data_bytes > MAX_EVENT_LINE_BYTES {
            bail!("Hyprland event line exceeds {MAX_EVENT_LINE_BYTES} bytes");
        }
        line.extend_from_slice(&available[..data_bytes]);
        reader.consume(consumed);
        if newline.is_some() {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return String::from_utf8(line)
                .map(Some)
                .context("Hyprland event is not UTF-8");
        }
    }
}

pub async fn run_bridge(
    ipc: HyprlandIpc,
    proxy: Core1Proxy<'_>,
    shortcut_spec: Option<ShortcutSpec>,
    logger: EventLogger,
) -> anyhow::Result<()> {
    let core = DbusCoreReporter { proxy };
    let mut events = match ipc.connect_events().await {
        Ok(events) => events,
        Err(error) => {
            let _ = core.clear().await;
            return Err(error);
        }
    };
    let mut reconciler = Reconciler::new();
    let mut keepalive = interval(KEEPALIVE_INTERVAL);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut focus_sync = interval(FOCUS_SYNC_INTERVAL);
    focus_sync.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut geometry_sync = interval(GEOMETRY_SYNC_INTERVAL);
    geometry_sync.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut focus_state = FocusSyncState::default();
    let mut geometry = GeometrySynchronizer::new();
    let mut diagnostics = BridgeDiagnostics::new(logger.clone());
    let shortcut_backend = ipc.shortcut_backend().await;
    match &shortcut_backend {
        Ok(backend) => logger.info("shortcut_backend", format_args!("backend={backend:?}")),
        Err(error) => logger.warn(
            "shortcut_backend_unavailable",
            format_args!("error={error}"),
        ),
    }
    let (mut shortcuts, shortcut_warning) =
        prepare_shortcut_reconcilers(shortcut_spec, shortcut_backend)?;
    if let Some(warning) = shortcut_warning {
        eprintln!("{warning}");
        logger.warn("shortcut_warning", format_args!("message={warning:?}"));
    }
    let mut latest_snapshot = CoreSnapshot::default();
    let debounce = sleep(DEBOUNCE_IDLE_DURATION);
    tokio::pin!(debounce);
    let mut reconciliation_pending = false;

    loop {
        tokio::select! {
            _ = keepalive.tick() => {
                latest_snapshot = reconcile_once(&ipc, &core, &mut reconciler).await?;
                for shortcut in &mut shortcuts {
                    shortcut.invalidate();
                }
                sync_shortcuts_once(&ipc, &latest_snapshot, &mut shortcuts).await?;
            }
            _ = focus_sync.tick() => {
                latest_snapshot = sync_focus_once(
                    &ipc,
                    &core,
                    &mut focus_state,
                    &mut diagnostics,
                ).await?;
                sync_shortcuts_once(&ipc, &latest_snapshot, &mut shortcuts).await?;
            }
            _ = geometry_sync.tick() => {
                sync_geometry_once(
                    &ipc,
                    &latest_snapshot,
                    &mut geometry,
                    &mut diagnostics,
                ).await?;
            }
            event = read_event_line(&mut events) => {
                match event {
                    Ok(Some(line)) if is_reconciliation_event(&line) => {
                        if line.starts_with("configreloaded>>")
                        {
                            for shortcut in &mut shortcuts {
                                shortcut.invalidate();
                            }
                        }
                        reconciliation_pending = true;
                        debounce.as_mut().reset(Instant::now() + EVENT_DEBOUNCE);
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        let _ = core.clear().await;
                        bail!("Hyprland event socket closed");
                    }
                    Err(error) => {
                        let _ = core.clear().await;
                        return Err(error);
                    }
                }
            }
            _ = &mut debounce, if reconciliation_pending => {
                reconciliation_pending = false;
                latest_snapshot = reconcile_once(&ipc, &core, &mut reconciler).await?;
                sync_shortcuts_once(&ipc, &latest_snapshot, &mut shortcuts).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Mutex};

    use anyhow::{Result, anyhow};
    use overcrow_config::ShortcutSettings;
    use overcrow_protocol::{CoreSnapshot, GameWindow, OverlayMode, Rect};
    use tokio::io::BufReader;

    use crate::{
        geometry::GeometrySynchronizer,
        ipc::MAX_EVENT_LINE_BYTES,
        model::{HyprMonitor, HyprWindow, OVERLAY_APP_ID, WindowReport},
        reconcile::Reconciler,
        shortcut::{
            BIND_DESCRIPTION, GlobalShortcut, HyprBinding, PORTAL_SHORTCUT_NAME, ShortcutBackend,
            ShortcutReconciler, ShortcutSpec,
        },
    };

    use super::{
        CoreReporter, DesktopIpc, DiagnosticRateLimiter, FocusSyncState, GEOMETRY_SYNC_INTERVAL,
        cleanup_commands, cleanup_focus_state_with, cleanup_runtime_state_with,
        decode_active_window, focus_commands, focus_commands_with_active, focus_plan,
        is_reconciliation_event, prepare_shortcut_reconciler, read_event_line, reconcile_once,
        sync_focus_once, sync_geometry_once, sync_shortcut_once, sync_shortcuts_once,
    };

    #[test]
    fn diagnostic_batches_are_limited_and_report_suppression() {
        let started = std::time::Instant::now();
        let mut limiter = DiagnosticRateLimiter::new(std::time::Duration::from_secs(1));

        assert_eq!(limiter.admit_at(started), Some(0));
        assert_eq!(
            limiter.admit_at(started + std::time::Duration::from_millis(33)),
            None
        );
        assert_eq!(
            limiter.admit_at(started + std::time::Duration::from_secs(1)),
            Some(1)
        );
    }

    struct RecordingCore {
        reports: Mutex<Vec<WindowReport>>,
        clears: Mutex<usize>,
        response: CoreSnapshot,
        snapshot_fails: bool,
    }

    impl RecordingCore {
        fn rejecting() -> Self {
            Self {
                reports: Mutex::new(Vec::new()),
                clears: Mutex::new(0),
                response: CoreSnapshot::default(),
                snapshot_fails: false,
            }
        }

        fn accepting_game() -> Self {
            Self {
                reports: Mutex::new(Vec::new()),
                clears: Mutex::new(0),
                response: CoreSnapshot {
                    active_game: Some(GameWindow {
                        pid: Some(136_279),
                        steam_app_id: Some(1_623_730),
                        app_id: Some("steam_app_1623730".to_owned()),
                        title: "Pal".to_owned(),
                        rect: Rect {
                            x: -10,
                            y: 36,
                            width: 2417,
                            height: 1680,
                        },
                        scale: 1.25,
                        backend: "wayland".to_owned(),
                    }),
                    ..CoreSnapshot::default()
                },
                snapshot_fails: false,
            }
        }
    }

    impl CoreReporter for RecordingCore {
        async fn report(&self, report: &WindowReport) -> Result<CoreSnapshot> {
            self.reports
                .lock()
                .expect("reports lock")
                .push(report.clone());
            Ok(self.response.clone())
        }

        async fn clear(&self) -> Result<CoreSnapshot> {
            *self.clears.lock().expect("clears lock") += 1;
            Ok(self.response.clone())
        }

        async fn snapshot(&self) -> Result<CoreSnapshot> {
            if self.snapshot_fails {
                return Err(anyhow!("snapshot unavailable"));
            }
            Ok(self.response.clone())
        }
    }

    struct FakeDesktop {
        active: Option<HyprWindow>,
        clients: Vec<HyprWindow>,
        monitors: Vec<HyprMonitor>,
        global_shortcuts: Vec<GlobalShortcut>,
        bindings: Vec<HyprBinding>,
        shortcut_backend: ShortcutBackend,
        dispatched: Mutex<Vec<String>>,
        active_queries: Mutex<usize>,
        client_queries: Mutex<usize>,
        monitor_queries: Mutex<usize>,
        global_shortcut_queries: Mutex<usize>,
        binding_queries: Mutex<usize>,
    }

    impl FakeDesktop {
        fn with_game_and_overlay() -> Self {
            Self::with_window_and_overlay("steam_app_1623730")
        }

        fn with_window_and_overlay(class: &str) -> Self {
            let game = sample_window("0x10", class);
            let overlay = sample_window("0x20", OVERLAY_APP_ID);
            Self {
                active: Some(game.clone()),
                clients: vec![game, overlay],
                monitors: vec![HyprMonitor { id: 0, scale: 1.25 }],
                global_shortcuts: Vec::new(),
                bindings: Vec::new(),
                shortcut_backend: ShortcutBackend::Compatibility,
                dispatched: Mutex::new(Vec::new()),
                active_queries: Mutex::new(0),
                client_queries: Mutex::new(0),
                monitor_queries: Mutex::new(0),
                global_shortcut_queries: Mutex::new(0),
                binding_queries: Mutex::new(0),
            }
        }

        fn without_active_window() -> Self {
            Self {
                active: None,
                clients: Vec::new(),
                monitors: vec![HyprMonitor { id: 0, scale: 1.0 }],
                global_shortcuts: Vec::new(),
                bindings: Vec::new(),
                shortcut_backend: ShortcutBackend::Compatibility,
                dispatched: Mutex::new(Vec::new()),
                active_queries: Mutex::new(0),
                client_queries: Mutex::new(0),
                monitor_queries: Mutex::new(0),
                global_shortcut_queries: Mutex::new(0),
                binding_queries: Mutex::new(0),
            }
        }

        fn with_portal_shortcut(mut self) -> Self {
            self.global_shortcuts.push(GlobalShortcut {
                name: PORTAL_SHORTCUT_NAME.to_owned(),
                description: "Open or close the OverCrow overlay".to_owned(),
            });
            self
        }

        fn with_owned_shortcut(mut self) -> Self {
            self.bindings.push(owned_shortcut_binding());
            self
        }

        fn dispatched(&self) -> Vec<String> {
            self.dispatched.lock().expect("dispatch lock").clone()
        }
    }

    impl DesktopIpc for FakeDesktop {
        async fn active_window(&self) -> Result<Option<HyprWindow>> {
            *self.active_queries.lock().expect("active query lock") += 1;
            Ok(self.active.clone())
        }

        async fn clients(&self) -> Result<Vec<HyprWindow>> {
            *self.client_queries.lock().expect("client query lock") += 1;
            Ok(self.clients.clone())
        }

        async fn monitors(&self) -> Result<Vec<HyprMonitor>> {
            *self.monitor_queries.lock().expect("monitor query lock") += 1;
            Ok(self.monitors.clone())
        }

        async fn global_shortcuts(&self) -> Result<Vec<GlobalShortcut>> {
            *self
                .global_shortcut_queries
                .lock()
                .expect("global shortcut query lock") += 1;
            Ok(self.global_shortcuts.clone())
        }

        async fn bindings(&self) -> Result<Vec<HyprBinding>> {
            *self.binding_queries.lock().expect("binding query lock") += 1;
            Ok(self.bindings.clone())
        }

        async fn shortcut_backend(&self) -> Result<ShortcutBackend> {
            Ok(self.shortcut_backend)
        }

        async fn dispatch(&self, command: &str) -> Result<()> {
            self.dispatched
                .lock()
                .expect("dispatch lock")
                .push(command.to_owned());
            Ok(())
        }
    }

    fn sample_window(address: &str, class: &str) -> HyprWindow {
        HyprWindow {
            address: address.to_owned(),
            mapped: true,
            hidden: false,
            at: [-10, 36],
            size: [2417, 1680],
            monitor: 0,
            class: class.to_owned(),
            title: "Pal".to_owned(),
            pid: 136_279,
            workspace: Some(crate::model::HyprWorkspace {
                id: 1,
                name: "1".to_owned(),
            }),
            tags: Vec::new(),
        }
    }

    fn default_shortcut_spec() -> ShortcutSpec {
        ShortcutSpec::from_settings(&ShortcutSettings {
            enabled: true,
            accelerator: "Meta+Alt+O".to_owned(),
        })
        .expect("valid shortcut")
        .expect("enabled shortcut")
    }

    fn owned_shortcut_binding() -> HyprBinding {
        HyprBinding {
            modmask: 72,
            key: "O".to_owned(),
            description: BIND_DESCRIPTION.to_owned(),
            dispatcher: "global".to_owned(),
            arg: PORTAL_SHORTCUT_NAME.to_owned(),
        }
    }

    fn foreign_shortcut_binding() -> HyprBinding {
        HyprBinding {
            description: "Foreign action".to_owned(),
            dispatcher: "exec".to_owned(),
            arg: "foreign".to_owned(),
            ..owned_shortcut_binding()
        }
    }

    fn manual_shortcut_specs() -> Vec<ShortcutSpec> {
        ShortcutSpec::manual_stopwatch()
            .expect("fixed manual shortcut specs are valid")
            .into_iter()
            .collect()
    }

    fn portal_action_for(spec: &ShortcutSpec) -> GlobalShortcut {
        GlobalShortcut {
            name: spec.portal_action().to_owned(),
            description: "registered by Core".to_owned(),
        }
    }

    fn owned_binding_for(spec: &ShortcutSpec) -> HyprBinding {
        HyprBinding {
            modmask: spec.modmask(),
            key: spec.key().to_owned(),
            description: spec.description().to_owned(),
            dispatcher: "global".to_owned(),
            arg: spec.portal_action().to_owned(),
        }
    }

    #[test]
    fn unknown_config_manager_disables_only_the_shortcut_adapter() {
        let backend = crate::ipc::detect_shortcut_backend("future manager response");

        let (shortcut, warning) = prepare_shortcut_reconciler(default_shortcut_spec(), backend)
            .expect("unknown manager is non-fatal");

        assert!(shortcut.is_none());
        let warning = warning.expect("unknown manager warning");
        assert!(warning.contains("shortcut integration is disabled"));
        assert!(warning.len() <= 320);
    }

    #[test]
    fn shortcut_backend_transport_failure_remains_fatal() {
        let result = prepare_shortcut_reconciler(
            default_shortcut_spec(),
            Err(anyhow!("Hyprland command socket failed")),
        );

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn active_game_installs_the_portal_dispatch_binding_once() {
        let desktop = FakeDesktop::with_game_and_overlay().with_portal_shortcut();
        let snapshot = RecordingCore::accepting_game().response;
        let mut shortcut =
            ShortcutReconciler::new(default_shortcut_spec(), ShortcutBackend::Compatibility);

        sync_shortcut_once(&desktop, &snapshot, &mut shortcut)
            .await
            .expect("initial shortcut sync");
        sync_shortcut_once(&desktop, &snapshot, &mut shortcut)
            .await
            .expect("stable shortcut sync");

        assert_eq!(
            desktop.dispatched(),
            [default_shortcut_spec().bind_request(ShortcutBackend::Compatibility)]
        );
    }

    #[tokio::test]
    async fn shortcut_waits_for_the_portal_then_retries() {
        let mut desktop = FakeDesktop::with_game_and_overlay();
        let snapshot = RecordingCore::accepting_game().response;
        let mut shortcut =
            ShortcutReconciler::new(default_shortcut_spec(), ShortcutBackend::Compatibility);

        sync_shortcut_once(&desktop, &snapshot, &mut shortcut)
            .await
            .expect("missing portal is non-fatal");
        assert!(desktop.dispatched().is_empty());

        desktop = desktop.with_portal_shortcut();
        sync_shortcut_once(&desktop, &snapshot, &mut shortcut)
            .await
            .expect("portal retry succeeds");
        assert_eq!(
            desktop.dispatched(),
            [default_shortcut_spec().bind_request(ShortcutBackend::Compatibility)]
        );
    }

    #[tokio::test]
    async fn passive_focus_loss_removes_the_owned_binding() {
        let desktop = FakeDesktop::with_game_and_overlay().with_owned_shortcut();
        let mut shortcut =
            ShortcutReconciler::new(default_shortcut_spec(), ShortcutBackend::Compatibility);
        shortcut.mark_bound();

        sync_shortcut_once(&desktop, &CoreSnapshot::default(), &mut shortcut)
            .await
            .expect("shortcut release succeeds");

        assert_eq!(
            desktop.dispatched(),
            [default_shortcut_spec().unbind_request(ShortcutBackend::Compatibility)]
        );
    }

    #[tokio::test]
    async fn interactive_snapshot_retains_the_shortcut() {
        let desktop = FakeDesktop::with_game_and_overlay()
            .with_portal_shortcut()
            .with_owned_shortcut();
        let mut snapshot = RecordingCore::accepting_game().response;
        snapshot.overlay_mode = OverlayMode::Interactive;
        let mut shortcut =
            ShortcutReconciler::new(default_shortcut_spec(), ShortcutBackend::Compatibility);

        sync_shortcut_once(&desktop, &snapshot, &mut shortcut)
            .await
            .expect("interactive shortcut sync");

        assert!(desktop.dispatched().is_empty());
        assert!(shortcut.is_owned());
    }

    #[tokio::test]
    async fn foreign_binding_conflict_never_dispatches() {
        let mut desktop = FakeDesktop::with_game_and_overlay().with_portal_shortcut();
        desktop.bindings.push(foreign_shortcut_binding());
        let snapshot = RecordingCore::accepting_game().response;
        let mut shortcut =
            ShortcutReconciler::new(default_shortcut_spec(), ShortcutBackend::Compatibility);

        sync_shortcut_once(&desktop, &snapshot, &mut shortcut)
            .await
            .expect("conflict is non-fatal");

        assert!(desktop.dispatched().is_empty());
    }

    #[tokio::test]
    async fn multi_action_cycle_probes_once_and_isolates_a_foreign_key_conflict() {
        let overlay = default_shortcut_spec();
        let manual = manual_shortcut_specs();
        let mut specs = vec![overlay.clone()];
        specs.extend(manual.iter().cloned());
        let mut reconcilers = specs
            .iter()
            .cloned()
            .map(|spec| ShortcutReconciler::new(spec, ShortcutBackend::Compatibility))
            .collect::<Vec<_>>();
        let mut desktop = FakeDesktop::with_game_and_overlay();
        desktop.global_shortcuts = specs.iter().map(portal_action_for).collect();
        desktop.bindings.push(HyprBinding {
            description: "Foreign stopwatch".to_owned(),
            dispatcher: "exec".to_owned(),
            arg: "foreign".to_owned(),
            ..owned_binding_for(&manual[0])
        });

        sync_shortcuts_once(
            &desktop,
            &RecordingCore::accepting_game().response,
            &mut reconcilers,
        )
        .await
        .expect("multi-action synchronization succeeds");

        assert_eq!(
            desktop.dispatched(),
            [
                overlay.bind_request(ShortcutBackend::Compatibility),
                manual[1].bind_request(ShortcutBackend::Compatibility),
            ]
        );
        assert_eq!(
            *desktop
                .global_shortcut_queries
                .lock()
                .expect("global query lock"),
            1
        );
        assert_eq!(
            *desktop.binding_queries.lock().expect("binding query lock"),
            1
        );
    }

    #[tokio::test]
    async fn runtime_cleanup_removes_one_exact_shortcut() {
        let desktop = FakeDesktop::with_game_and_overlay().with_owned_shortcut();

        cleanup_runtime_state_with(&desktop)
            .await
            .expect("runtime cleanup succeeds");

        assert_eq!(
            desktop.dispatched(),
            [default_shortcut_spec().unbind_request(ShortcutBackend::Compatibility)]
        );
    }

    #[tokio::test]
    async fn runtime_cleanup_preserves_an_ambiguous_accelerator() {
        let mut desktop = FakeDesktop::with_game_and_overlay().with_owned_shortcut();
        desktop.bindings.push(foreign_shortcut_binding());

        cleanup_runtime_state_with(&desktop)
            .await
            .expect("ambiguous cleanup fails closed");

        assert!(desktop.dispatched().is_empty());
    }

    #[tokio::test]
    async fn runtime_cleanup_removes_only_exact_known_action_description_pairs() {
        let overlay = default_shortcut_spec();
        let manual = manual_shortcut_specs();
        let specs = [&overlay, &manual[0], &manual[1]];
        let mut desktop = FakeDesktop::with_game_and_overlay();
        desktop.bindings = specs.iter().map(|spec| owned_binding_for(spec)).collect();
        desktop.bindings.push(HyprBinding {
            modmask: 64,
            key: "X".to_owned(),
            description: "OverCrow manual stopwatch reset".to_owned(),
            dispatcher: "exec".to_owned(),
            arg: ":reset-manual-stopwatch".to_owned(),
        });

        cleanup_runtime_state_with(&desktop)
            .await
            .expect("known shortcut cleanup succeeds");

        assert_eq!(
            desktop.dispatched(),
            specs
                .iter()
                .map(|spec| spec.unbind_request(ShortcutBackend::Compatibility))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn geometry_sync_uses_the_thirty_hertz_interval() {
        assert_eq!(GEOMETRY_SYNC_INTERVAL, std::time::Duration::from_millis(33));
    }

    #[tokio::test]
    async fn geometry_sync_is_idle_without_a_confirmed_game() {
        let desktop = FakeDesktop::with_game_and_overlay();
        let mut geometry = GeometrySynchronizer::new();
        let mut diagnostics = super::BridgeDiagnostics::disabled();

        sync_geometry_once(
            &desktop,
            &CoreSnapshot::default(),
            &mut geometry,
            &mut diagnostics,
        )
        .await
        .expect("idle geometry synchronization succeeds");

        assert_eq!(*desktop.active_queries.lock().expect("query lock"), 0);
        assert_eq!(*desktop.client_queries.lock().expect("query lock"), 0);
        assert_eq!(*desktop.monitor_queries.lock().expect("query lock"), 0);
        assert!(desktop.dispatched.lock().expect("dispatch lock").is_empty());
    }

    #[tokio::test]
    async fn geometry_sync_dispatches_only_the_planned_exact_commands() {
        let mut desktop = FakeDesktop::with_game_and_overlay();
        desktop
            .clients
            .iter_mut()
            .find(|window| window.class == OVERLAY_APP_ID)
            .expect("overlay client")
            .at = [140, 90];
        let snapshot = RecordingCore::accepting_game().response;
        let mut geometry = GeometrySynchronizer::new();
        let mut diagnostics = super::BridgeDiagnostics::disabled();

        sync_geometry_once(&desktop, &snapshot, &mut geometry, &mut diagnostics)
            .await
            .expect("geometry synchronization succeeds");

        assert_eq!(*desktop.active_queries.lock().expect("query lock"), 1);
        assert_eq!(*desktop.client_queries.lock().expect("query lock"), 1);
        assert_eq!(*desktop.monitor_queries.lock().expect("query lock"), 0);
        assert_eq!(
            *desktop.dispatched.lock().expect("dispatch lock"),
            [
                "dispatch movetoworkspacesilent 1,address:0x20",
                "dispatch resizewindowpixel exact 2417 1680,address:0x20",
                "dispatch movewindowpixel exact -10 36,address:0x20",
                "dispatch alterzorder top,address:0x20",
            ]
        );
    }

    #[test]
    fn interactive_transition_waits_for_the_tag_then_focuses_once() {
        let mut game = sample_window("0x10", "steam_app_1623730");
        let mut overlay = sample_window("0x20", OVERLAY_APP_ID);
        let lookalike = sample_window("0x22", "io.github.overcrow.Overlay.fake");
        let mut snapshot = RecordingCore::accepting_game().response;
        snapshot.overlay_mode = OverlayMode::Interactive;
        let mut state = FocusSyncState::default();

        assert_eq!(
            focus_commands(
                &[game.clone(), overlay.clone(), lookalike.clone()],
                &snapshot,
                &mut state
            ),
            [
                "dispatch tagwindow +overcrow-game-input-blocked address:0x10",
                "dispatch tagwindow +overcrow-interactive address:0x20",
            ]
        );

        game.tags.push("overcrow-game-input-blocked".to_owned());
        overlay.tags.push("overcrow-interactive".to_owned());
        assert_eq!(
            focus_commands(
                &[game.clone(), overlay.clone(), lookalike],
                &snapshot,
                &mut state
            ),
            ["dispatch focuswindow address:0x20"]
        );
        // Overlay already focused: do not focus again.
        assert!(
            focus_commands_with_active(
                &[game, overlay.clone()],
                Some(&overlay),
                &snapshot,
                &mut state
            )
            .is_empty()
        );
    }

    #[test]
    fn stable_interactive_mode_does_not_steal_focus_from_other_apps() {
        let mut tagged_game = sample_window("0x10", "steam_app_1623730");
        tagged_game
            .tags
            .push("overcrow-game-input-blocked".to_owned());
        let mut tagged_overlay = sample_window("0x21", OVERLAY_APP_ID);
        tagged_overlay.tags.push("overcrow-interactive".to_owned());
        let mut other = sample_window("0x30", "code");
        other.title = "editor".to_owned();
        let mut snapshot = RecordingCore::accepting_game().response;
        snapshot.overlay_mode = OverlayMode::Interactive;
        let mut state = FocusSyncState::default();

        assert_eq!(
            focus_commands(
                &[tagged_game.clone(), tagged_overlay.clone()],
                &snapshot,
                &mut state
            ),
            ["dispatch focuswindow address:0x21"]
        );
        // Overlay already focused: leave it alone.
        assert!(
            focus_commands_with_active(
                &[tagged_game.clone(), tagged_overlay.clone()],
                Some(&tagged_overlay),
                &snapshot,
                &mut state
            )
            .is_empty()
        );
        // Another application intentionally focused: do not steal.
        assert!(
            focus_commands_with_active(
                &[tagged_game.clone(), tagged_overlay.clone(), other.clone()],
                Some(&other),
                &snapshot,
                &mut state
            )
            .is_empty()
        );
    }

    #[test]
    fn fresh_interactive_state_does_not_steal_focus_from_an_unrelated_app() {
        let mut tagged_game = sample_window("0x10", "steam_app_1623730");
        tagged_game
            .tags
            .push("overcrow-game-input-blocked".to_owned());
        let mut tagged_overlay = sample_window("0x21", OVERLAY_APP_ID);
        tagged_overlay.tags.push("overcrow-interactive".to_owned());
        let mut editor = sample_window("0x30", "code");
        editor.title = "editor".to_owned();
        let mut snapshot = RecordingCore::accepting_game().response;
        snapshot.overlay_mode = OverlayMode::Interactive;
        let mut state = FocusSyncState::default();

        assert!(
            focus_commands_with_active(
                &[tagged_game, tagged_overlay, editor.clone()],
                Some(&editor),
                &snapshot,
                &mut state,
            )
            .is_empty()
        );
    }

    #[test]
    fn interactive_mode_recovers_focus_after_global_shortcut_focus_loss() {
        let mut tagged_game = sample_window("0x10", "steam_app_1623730");
        tagged_game
            .tags
            .push("overcrow-game-input-blocked".to_owned());
        let mut tagged_overlay = sample_window("0x21", OVERLAY_APP_ID);
        tagged_overlay.tags.push("overcrow-interactive".to_owned());
        let mut snapshot = RecordingCore::accepting_game().response;
        snapshot.overlay_mode = OverlayMode::Interactive;
        let mut state = FocusSyncState {
            observed_mode: Some(OverlayMode::Interactive),
            interactive_focus_pending: false,
            ..FocusSyncState::default()
        };

        // Without a pending transition or validated active game, no-focus does
        // not establish authority to take focus.
        assert!(
            focus_commands_with_active(
                &[tagged_game.clone(), tagged_overlay.clone()],
                None,
                &snapshot,
                &mut state
            )
            .is_empty()
        );
        // Focus stuck on the input-blocked game: recover overlay.
        assert_eq!(
            focus_commands_with_active(
                &[tagged_game.clone(), tagged_overlay.clone()],
                Some(&tagged_game),
                &snapshot,
                &mut state
            ),
            ["dispatch focuswindow address:0x21"]
        );
    }

    #[test]
    fn passive_transition_removes_tag_then_restores_the_validated_game() {
        let mut game = sample_window("0x10", "steam_app_1623730");
        game.tags.push("overcrow-game-input-blocked".to_owned());
        let mut tagged_overlay = sample_window("0x21", OVERLAY_APP_ID);
        tagged_overlay.tags.push("overcrow-interactive".to_owned());
        let snapshot = RecordingCore::accepting_game().response;
        let mut state = FocusSyncState {
            observed_mode: Some(OverlayMode::Interactive),
            ..FocusSyncState::default()
        };

        assert_eq!(
            focus_commands(
                &[game.clone(), tagged_overlay.clone()],
                &snapshot,
                &mut state
            ),
            [
                "dispatch tagwindow -overcrow-game-input-blocked address:0x10",
                "dispatch tagwindow -overcrow-interactive address:0x21",
            ]
        );

        game.tags.clear();
        tagged_overlay.tags.clear();
        assert_eq!(
            focus_commands(&[game, tagged_overlay], &snapshot, &mut state),
            ["dispatch focuswindow address:0x10"]
        );
    }

    #[test]
    fn passive_transition_does_not_focus_a_mismatched_or_ambiguous_game() {
        let mut wrong_class = sample_window("0x10", "code");
        wrong_class.title = "VS Code".to_owned();
        let matching_game = sample_window("0x11", "steam_app_1623730");
        let duplicate_game = sample_window("0x12", "steam_app_1623730");
        let mut tagged_overlay = sample_window("0x21", OVERLAY_APP_ID);
        tagged_overlay.tags.push("overcrow-interactive".to_owned());
        let snapshot = RecordingCore::accepting_game().response;
        let mut state = FocusSyncState {
            observed_mode: Some(OverlayMode::Interactive),
            ..FocusSyncState::default()
        };

        assert_eq!(
            focus_commands(
                &[wrong_class, tagged_overlay.clone()],
                &snapshot,
                &mut state
            ),
            ["dispatch tagwindow -overcrow-interactive address:0x21"]
        );

        state.observed_mode = Some(OverlayMode::Interactive);
        assert_eq!(
            focus_commands(
                &[matching_game, duplicate_game, tagged_overlay],
                &snapshot,
                &mut state
            ),
            ["dispatch tagwindow -overcrow-interactive address:0x21"]
        );
    }

    #[test]
    fn invalid_interactive_targets_clear_core_and_remove_stale_tags() {
        let mut first_game = sample_window("0x10", "steam_app_1623730");
        first_game
            .tags
            .push("overcrow-game-input-blocked".to_owned());
        let duplicate_game = sample_window("0x11", "steam_app_1623730");
        let mut overlay = sample_window("0x20", OVERLAY_APP_ID);
        overlay.tags.push("overcrow-interactive".to_owned());
        let mut snapshot = RecordingCore::accepting_game().response;
        snapshot.overlay_mode = OverlayMode::Interactive;
        let mut state = FocusSyncState::default();

        let plan = focus_plan(
            &[first_game, duplicate_game, overlay],
            None,
            &snapshot,
            &mut state,
        );

        assert!(plan.clear_core);
        assert_eq!(
            plan.commands,
            [
                "dispatch tagwindow -overcrow-game-input-blocked address:0x10",
                "dispatch tagwindow -overcrow-interactive address:0x20",
            ]
        );
        assert!(
            plan.commands
                .iter()
                .all(|command| !command.contains("focuswindow"))
        );
    }

    #[test]
    fn cleanup_removes_only_reserved_tags_from_valid_addresses() {
        let mut game = sample_window("0x10", "steam_app_1623730");
        game.tags = vec![
            "unrelated".to_owned(),
            "overcrow-game-input-blocked".to_owned(),
        ];
        let mut overlay = sample_window("0x20", OVERLAY_APP_ID);
        overlay.tags = vec![
            "overcrow-game-input-blocked".to_owned(),
            "overcrow-interactive".to_owned(),
        ];
        let mut malformed = sample_window("not-an-address", "code");
        malformed.tags.push("overcrow-interactive".to_owned());
        let clean = sample_window("0x30", "code");

        assert_eq!(
            cleanup_commands(&[game, overlay, malformed, clean]),
            [
                "dispatch tagwindow -overcrow-game-input-blocked address:0x10",
                "dispatch tagwindow -overcrow-game-input-blocked address:0x20",
                "dispatch tagwindow -overcrow-interactive address:0x20",
            ]
        );
    }

    #[tokio::test]
    async fn cleanup_queries_once_and_dispatches_reserved_tag_removals() {
        let mut desktop = FakeDesktop::with_game_and_overlay();
        desktop.clients[0]
            .tags
            .push("overcrow-game-input-blocked".to_owned());
        desktop.clients[1]
            .tags
            .push("overcrow-interactive".to_owned());

        cleanup_focus_state_with(&desktop)
            .await
            .expect("cleanup succeeds");

        assert_eq!(*desktop.client_queries.lock().expect("query lock"), 1);
        assert_eq!(
            *desktop.dispatched.lock().expect("dispatch lock"),
            [
                "dispatch tagwindow -overcrow-game-input-blocked address:0x10",
                "dispatch tagwindow -overcrow-interactive address:0x20",
            ]
        );
    }

    #[tokio::test]
    async fn invalid_interactive_targets_request_core_clear() {
        let mut desktop = FakeDesktop::with_game_and_overlay();
        desktop
            .clients
            .push(sample_window("0x11", "steam_app_1623730"));
        let mut core = RecordingCore::accepting_game();
        core.response.overlay_mode = OverlayMode::Interactive;
        let mut state = FocusSyncState::default();
        let mut diagnostics = super::BridgeDiagnostics::disabled();

        sync_focus_once(&desktop, &core, &mut state, &mut diagnostics)
            .await
            .expect("invalid Interactive state clears safely");

        assert_eq!(*core.clears.lock().expect("clears lock"), 1);
        assert!(desktop.dispatched.lock().expect("dispatch lock").is_empty());
    }

    #[tokio::test]
    async fn reportable_focus_reports_without_competing_geometry_dispatches() {
        let desktop = FakeDesktop::with_game_and_overlay();
        let core = RecordingCore::accepting_game();

        reconcile_once(&desktop, &core, &mut Reconciler::new())
            .await
            .expect("reconciliation succeeds");

        assert_eq!(core.reports.lock().expect("reports lock").len(), 1);
        assert_eq!(*core.clears.lock().expect("clears lock"), 0);
        assert!(desktop.dispatched.lock().expect("dispatch lock").is_empty());
    }

    #[tokio::test]
    async fn core_rejected_application_is_reported_without_overlay_placement() {
        let desktop = FakeDesktop::with_window_and_overlay("code");
        let core = RecordingCore::rejecting();

        reconcile_once(&desktop, &core, &mut Reconciler::new())
            .await
            .expect("classification rejection is a valid reconciliation");

        assert_eq!(core.reports.lock().expect("reports lock").len(), 1);
        assert!(desktop.dispatched.lock().expect("dispatch lock").is_empty());
    }

    #[tokio::test]
    async fn reconciliation_leaves_geometry_to_the_synchronizer() {
        let desktop = FakeDesktop::with_game_and_overlay();
        let mut core = RecordingCore::accepting_game();
        core.response
            .active_game
            .as_mut()
            .expect("accepted game")
            .rect = Rect {
            x: 40,
            y: 50,
            width: 800,
            height: 600,
        };

        reconcile_once(&desktop, &core, &mut Reconciler::new())
            .await
            .expect("reconciliation succeeds");

        assert!(desktop.dispatched.lock().expect("dispatch lock").is_empty());
    }

    #[tokio::test]
    async fn interactive_core_mode_preserves_game_across_unrelated_focus() {
        let game_desktop = FakeDesktop::with_game_and_overlay();
        let game = sample_window("0x10", "steam_app_1623730");
        let browser = sample_window("0x30", "code");
        let overlay = sample_window("0x20", OVERLAY_APP_ID);
        let browser_desktop = FakeDesktop {
            active: Some(browser.clone()),
            clients: vec![game, browser, overlay],
            monitors: vec![HyprMonitor { id: 0, scale: 1.25 }],
            global_shortcuts: Vec::new(),
            bindings: Vec::new(),
            shortcut_backend: ShortcutBackend::Compatibility,
            dispatched: Mutex::new(Vec::new()),
            active_queries: Mutex::new(0),
            client_queries: Mutex::new(0),
            monitor_queries: Mutex::new(0),
            global_shortcut_queries: Mutex::new(0),
            binding_queries: Mutex::new(0),
        };
        let mut core = RecordingCore::accepting_game();
        let mut reconciler = Reconciler::new();
        reconcile_once(&game_desktop, &core, &mut reconciler)
            .await
            .expect("initial game report");

        core.response.overlay_mode = OverlayMode::Interactive;
        core.reports.lock().expect("reports lock").clear();
        reconcile_once(&browser_desktop, &core, &mut reconciler)
            .await
            .expect("interactive retention");

        assert_eq!(
            core.reports.lock().expect("reports lock")[0].app_id,
            "steam_app_1623730"
        );
    }

    #[tokio::test]
    async fn focus_sync_adds_the_interactive_tag_from_core_state() {
        let desktop = FakeDesktop::with_game_and_overlay();
        let mut core = RecordingCore::accepting_game();
        core.response.overlay_mode = OverlayMode::Interactive;
        let mut state = FocusSyncState::default();
        let mut diagnostics = super::BridgeDiagnostics::disabled();

        sync_focus_once(&desktop, &core, &mut state, &mut diagnostics)
            .await
            .expect("focus synchronization succeeds");

        assert_eq!(
            *desktop.dispatched.lock().expect("dispatch lock"),
            [
                "dispatch tagwindow +overcrow-game-input-blocked address:0x10",
                "dispatch tagwindow +overcrow-interactive address:0x20",
            ]
        );
    }

    #[tokio::test]
    async fn failed_core_snapshot_attempts_to_remove_interactive_focus_tag() {
        let mut desktop = FakeDesktop::with_game_and_overlay();
        desktop
            .clients
            .iter_mut()
            .find(|window| window.class == OVERLAY_APP_ID)
            .expect("overlay client")
            .tags
            .push("overcrow-interactive".to_owned());
        let mut core = RecordingCore::rejecting();
        core.snapshot_fails = true;
        let mut state = FocusSyncState::default();
        let mut diagnostics = super::BridgeDiagnostics::disabled();

        assert!(
            sync_focus_once(&desktop, &core, &mut state, &mut diagnostics)
                .await
                .is_err()
        );
        assert_eq!(
            *desktop.dispatched.lock().expect("dispatch lock"),
            ["dispatch tagwindow -overcrow-interactive address:0x20"]
        );
    }

    #[tokio::test]
    async fn invalid_desktop_state_clears_without_dispatch() {
        let desktop = FakeDesktop::without_active_window();
        let core = RecordingCore::rejecting();

        reconcile_once(&desktop, &core, &mut Reconciler::new())
            .await
            .expect("fail-closed reconciliation succeeds");

        assert_eq!(*core.clears.lock().expect("clears lock"), 1);
        assert!(core.reports.lock().expect("reports lock").is_empty());
        assert!(desktop.dispatched.lock().expect("dispatch lock").is_empty());
    }

    #[test]
    fn empty_active_window_object_means_no_focus() {
        assert_eq!(decode_active_window(serde_json::json!({})).unwrap(), None);
        let decoded = decode_active_window(serde_json::json!({
            "address":"0x10", "mapped":true, "hidden":false,
            "at":[0,0], "size":[100,100], "monitor":0,
            "class":"game", "title":"Game", "pid":42
        }))
        .unwrap();
        assert_eq!(decoded.expect("window").address, "0x10");
        assert!(decode_active_window(serde_json::json!({"address": 42})).is_err());
    }

    #[test]
    fn accepts_only_reconciliation_events() {
        for line in [
            "activewindowv2>>55aabb",
            "openwindow>>55aabb,1,class,title",
            "closewindow>>55aabb",
            "fullscreen>>1",
            "workspacev2>>1,1",
            "minimized>>55aabb,1",
            "configreloaded>>",
        ] {
            assert!(is_reconciliation_event(line), "rejected {line}");
        }
        assert!(!is_reconciliation_event("bell>>ignored"));
        assert!(!is_reconciliation_event("missing-separator"));
    }

    #[tokio::test]
    async fn event_reader_rejects_oversized_lines_before_unbounded_growth() {
        let mut bytes = vec![b'x'; MAX_EVENT_LINE_BYTES + 1];
        bytes.push(b'\n');
        let mut reader = BufReader::new(Cursor::new(bytes));

        assert!(read_event_line(&mut reader).await.is_err());
    }
}
