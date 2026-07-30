import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const CORE_BUS_NAME = 'io.github.overcrow.Core1';
const CORE_PATH = '/io/github/overcrow/Core1';
const CORE_INTERFACE = 'io.github.overcrow.Core1';
const OVERLAY_APP_ID = 'io.github.overcrow.Overlay';
const KEEPALIVE_MS = 2000;
const NULL_FOCUS_GRACE_MS = 500;
const CANDIDATE_RETRY_MS = 500;
const MAX_CANDIDATE_RETRIES = 6;
const DBUS_TIMEOUT_MS = 1500;
const MAX_SNAPSHOT_BYTES = 65536;
const MAX_SHORTCUT_JSON_BYTES = 1024;
const MAX_SHORTCUTS = 3;
const MAX_SHORTCUT_TEXT_CODE_UNITS = 128;
const MAX_APP_ID_CODE_UNITS = 256;
const MAX_TITLE_CODE_UNITS = 256;
const INT32_MIN = -2147483648;
const INT32_MAX = 2147483647;

const SHORTCUT_METHODS = new Map([
    ['toggle-overlay', 'ToggleOverlay'],
    ['toggle-manual-stopwatch', 'ToggleManualStopwatch'],
    ['reset-manual-stopwatch', 'ResetManualStopwatch'],
]);

function safeCall(callback, fallback = null) {
    try {
        return callback();
    } catch {
        return fallback;
    }
}

function boundedIdentity(value) {
    if (typeof value !== 'string')
        return null;
    const trimmed = value.trim();
    if (
        trimmed.length === 0 ||
        trimmed.length > MAX_APP_ID_CODE_UNITS ||
        trimmed.includes('\0')
    ) {
        return null;
    }
    return trimmed;
}

function boundedTitle(value) {
    if (typeof value !== 'string')
        return '';
    return value.replaceAll('\0', '').slice(0, MAX_TITLE_CODE_UNITS);
}

function finiteInteger(value, minimum, maximum) {
    return Number.isInteger(value) && value >= minimum && value <= maximum
        ? value
        : null;
}

function sameRect(left, right) {
    return (
        left !== null &&
        right !== null &&
        left.x === right.x &&
        left.y === right.y &&
        left.width === right.width &&
        left.height === right.height
    );
}

function windowIdentity(window) {
    if (!window)
        return null;
    for (const method of [
        'get_sandboxed_app_id',
        'get_gtk_application_id',
        'get_wm_class',
    ]) {
        const identity = boundedIdentity(
            safeCall(() => window[method]?.call(window)),
        );
        if (identity !== null)
            return identity;
    }
    return null;
}

function isOverlayWindow(window) {
    return windowIdentity(window) === OVERLAY_APP_ID;
}

function gtkAccelerator(accelerator) {
    if (
        typeof accelerator !== 'string' ||
        accelerator.length === 0 ||
        accelerator.length > MAX_SHORTCUT_TEXT_CODE_UNITS
    ) {
        return null;
    }
    const parts = accelerator.split('+');
    if (parts.length < 2 || parts.length > 5)
        return null;
    const key = parts.pop();
    if (!/^[A-Za-z0-9]$/.test(key))
        return null;

    const modifiers = new Map([
        ['Meta', {rank: 0, gtk: '<Super>'}],
        ['Ctrl', {rank: 1, gtk: '<Control>'}],
        ['Alt', {rank: 2, gtk: '<Alt>'}],
        ['Shift', {rank: 3, gtk: '<Shift>'}],
    ]);
    let previousRank = -1;
    let normalized = '';
    for (const part of parts) {
        const modifier = modifiers.get(part);
        if (!modifier || modifier.rank <= previousRank)
            return null;
        previousRank = modifier.rank;
        normalized += modifier.gtk;
    }
    return `${normalized}${key.toUpperCase()}`;
}

function parseShortcutDefinitions(json) {
    if (
        typeof json !== 'string' ||
        json.length > MAX_SHORTCUT_JSON_BYTES
    ) {
        return null;
    }
    let values;
    try {
        values = JSON.parse(json);
    } catch {
        return null;
    }
    if (!Array.isArray(values) || values.length > MAX_SHORTCUTS)
        return null;

    const ids = new Set();
    const accelerators = new Set();
    const definitions = [];
    for (const value of values) {
        if (
            value === null ||
            typeof value !== 'object' ||
            Array.isArray(value) ||
            Object.keys(value).sort().join(',') !== 'accelerator,id' ||
            !SHORTCUT_METHODS.has(value.id) ||
            ids.has(value.id)
        ) {
            return null;
        }
        const accelerator = gtkAccelerator(value.accelerator);
        if (accelerator === null || accelerators.has(accelerator))
            return null;
        ids.add(value.id);
        accelerators.add(accelerator);
        definitions.push({id: value.id, accelerator});
    }
    return definitions;
}

export class GnomeBridgeRuntime {
    constructor({Gio: gio, GLib: glib, Meta: meta, display, dbus}) {
        this._Gio = gio;
        this._GLib = glib;
        this._Meta = meta;
        this._display = display;
        this._dbus = dbus;

        this._started = false;
        this._coreOwner = null;
        this._generation = 0;
        this._nameWatchId = 0;
        this._snapshotSignalId = 0;
        this._displaySignals = [];
        this._windowSignals = new Map();
        this._sources = new Set();
        this._focusGraceSource = 0;
        this._candidateRetrySource = 0;
        this._overlayWindow = null;
        this._candidateWindow = null;
        this._gameWindow = null;
        this._mode = 'Passive';
        this._lastSnapshotRevision = -1;
        this._lastReportSignature = null;
        this._clearAuthorityFence = false;
        this._windowRequestInFlight = false;
        this._pendingWindowRequest = null;
        this._windowIntentSerial = 0;
        this._actionToShortcut = new Map();
        this._shortcutSignature = null;
        this._shortcutRequestSerial = 0;
        this._reportedAvailability = null;
        this._placingOverlay = false;
    }

    start() {
        if (this._started)
            return;
        this._started = true;
        this._nameWatchId = this._Gio.bus_watch_name(
            this._Gio.BusType.SESSION,
            CORE_BUS_NAME,
            this._Gio.BusNameWatcherFlags.NONE,
            (_connection, _name, owner) => this._onCoreAppeared(owner),
            () => this._onCoreVanished(),
        );
    }

    stop() {
        if (!this._started)
            return;
        if (this._coreOwner !== null) {
            this._callCore('ClearWindow', '()', [], '(s)');
            this._reportShortcutAvailability('disabled');
        }
        this._detachCore();
        if (this._nameWatchId !== 0) {
            this._Gio.bus_unwatch_name(this._nameWatchId);
            this._nameWatchId = 0;
        }
        this._started = false;
    }

    _onCoreAppeared(owner) {
        if (!this._started || boundedIdentity(owner) === null)
            return;
        this._detachCore();
        this._coreOwner = owner;
        this._generation += 1;
        this._snapshotSignalId = this._dbus.signal_subscribe(
            CORE_BUS_NAME,
            CORE_INTERFACE,
            'SnapshotChanged',
            CORE_PATH,
            null,
            this._Gio.DBusSignalFlags.NONE,
            (_connection, _sender, _path, _interface, _signal, parameters) => {
                const [json] = safeCall(() => parameters.deepUnpack(), []);
                this._applySnapshotJson(json);
            },
        );
        this._attachCompositor();
        this._addTimeout(KEEPALIVE_MS, true, () => {
            const focus = this._display.get_focus_window();
            if (
                this._gameWindow !== null &&
                (focus === this._gameWindow || focus === this._overlayWindow)
            ) {
                this._requestReport(this._gameWindow, true);
            }
            return true;
        });
        this._requestSnapshot(() => {
            this._handleFocusWindow(this._display.get_focus_window());
        });
    }

    _onCoreVanished() {
        if (this._coreOwner !== null)
            this._detachCore();
    }

    _detachCore() {
        this._generation += 1;
        if (this._snapshotSignalId !== 0) {
            this._dbus.signal_unsubscribe(this._snapshotSignalId);
            this._snapshotSignalId = 0;
        }
        for (const id of this._displaySignals)
            safeCall(() => this._display.disconnect(id));
        this._displaySignals = [];
        for (const [window, ids] of this._windowSignals) {
            for (const id of ids)
                safeCall(() => window.disconnect(id));
        }
        this._windowSignals.clear();
        for (const id of [...this._sources])
            this._removeSource(id);
        this._releaseShortcuts();
        this._releaseOverlayAuthority();

        this._coreOwner = null;
        this._overlayWindow = null;
        this._candidateWindow = null;
        this._gameWindow = null;
        this._mode = 'Passive';
        this._lastSnapshotRevision = -1;
        this._lastReportSignature = null;
        this._clearAuthorityFence = false;
        this._windowRequestInFlight = false;
        this._pendingWindowRequest = null;
        this._windowIntentSerial += 1;
        this._shortcutRequestSerial += 1;
        this._reportedAvailability = null;
        this._focusGraceSource = 0;
        this._candidateRetrySource = 0;
    }

    _attachCompositor() {
        this._connectDisplay('notify::focus-window', () =>
            this._onFocusChanged(),
        );
        this._connectDisplay('window-created', (_display, window) => {
            this._attachWindow(window);
            this._onWindowIdentityChanged(window);
        });
        this._connectDisplay('restacked', () => this._placeOverlay());
        this._connectDisplay(
            'accelerator-activated',
            (_display, actionId) => this._onAccelerator(actionId),
        );

        for (const window of this._display.list_all_windows()) {
            this._attachWindow(window);
            if (isOverlayWindow(window))
                this._overlayWindow = window;
        }
    }

    _connectDisplay(name, callback) {
        const id = safeCall(() => this._display.connect(name, callback), 0);
        if (id !== 0)
            this._displaySignals.push(id);
    }

    _attachWindow(window) {
        if (!window || this._windowSignals.has(window))
            return;
        const ids = [];
        const connect = (name, callback) => {
            const id = safeCall(() => window.connect(name, callback), 0);
            if (id !== 0)
                ids.push(id);
        };
        connect('position-changed', () => this._onWindowChanged(window));
        connect('size-changed', () => this._onWindowChanged(window));
        connect('workspace-changed', () => this._onWindowChanged(window));
        connect('notify::minimized', () => this._onWindowChanged(window));
        connect('notify::wm-class', () =>
            this._onWindowIdentityChanged(window),
        );
        connect('notify::gtk-application-id', () =>
            this._onWindowIdentityChanged(window),
        );
        connect('shown', () => this._onWindowIdentityChanged(window));
        connect('unmanaged', () => this._onWindowUnmanaged(window));
        this._windowSignals.set(window, ids);
    }

    _onWindowIdentityChanged(window) {
        if (isOverlayWindow(window)) {
            this._overlayWindow = window;
            this._placeOverlay();
            return;
        }
        if (window !== this._display.get_focus_window())
            return;
        this._cancelCandidateRetry();
        this._candidateWindow = window;
        this._requestReport(window, true, MAX_CANDIDATE_RETRIES);
    }

    _onWindowChanged(window) {
        if (window === this._overlayWindow) {
            if (!this._placingOverlay)
                this._placeOverlay();
            return;
        }
        if (
            window === this._gameWindow ||
            window === this._candidateWindow
        ) {
            this._requestReport(window, false);
            this._placeOverlay();
        }
    }

    _onWindowUnmanaged(window) {
        const ids = this._windowSignals.get(window) ?? [];
        for (const id of ids)
            safeCall(() => window.disconnect(id));
        this._windowSignals.delete(window);
        if (window === this._overlayWindow) {
            this._overlayWindow = null;
            return;
        }
        if (
            window === this._candidateWindow ||
            window === this._gameWindow
        ) {
            this._candidateWindow = null;
            this._gameWindow = null;
            this._requestClear();
        }
    }

    _onFocusChanged() {
        this._cancelFocusGrace();
        const window = this._display.get_focus_window();
        if (window === null) {
            this._focusGraceSource = this._addTimeout(
                NULL_FOCUS_GRACE_MS,
                false,
                () => {
                    this._focusGraceSource = 0;
                    const settled = this._display.get_focus_window();
                    if (settled === null)
                        this._requestClear();
                    else
                        this._handleFocusWindow(settled);
                    return false;
                },
            );
            return;
        }
        this._handleFocusWindow(window);
    }

    _handleFocusWindow(window) {
        if (window === null || this._coreOwner === null)
            return;
        if (isOverlayWindow(window)) {
            this._overlayWindow = window;
            this._placeOverlay();
            if (this._mode === 'Passive' && this._gameWindow !== null)
                this._activate(this._gameWindow);
            else if (this._gameWindow !== null)
                this._requestReport(this._gameWindow, true);
            return;
        }
        this._cancelCandidateRetry();
        if (
            this._mode === 'Interactive' &&
            window === this._gameWindow &&
            this._overlayWindow !== null
        ) {
            this._activate(this._overlayWindow);
            return;
        }
        if (this._gameWindow !== null && window !== this._gameWindow)
            this._applyPassiveState();
        this._candidateWindow = window;
        this._requestReport(window, true, MAX_CANDIDATE_RETRIES);
    }

    _normalizedWindow(window) {
        if (
            !window ||
            isOverlayWindow(window) ||
            window.minimized === true ||
            safeCall(() => window.is_hidden(), false) === true ||
            safeCall(() => window.is_override_redirect(), false) === true
        ) {
            return null;
        }
        const pid = finiteInteger(
            safeCall(() => window.get_pid()),
            1,
            INT32_MAX,
        );
        const identity = windowIdentity(window);
        const rect = safeCall(() => window.get_frame_rect());
        const monitor = finiteInteger(
            safeCall(() => window.get_monitor()),
            0,
            INT32_MAX,
        );
        if (pid === null || identity === null || !rect || monitor === null)
            return null;
        const x = finiteInteger(rect.x, INT32_MIN, INT32_MAX);
        const y = finiteInteger(rect.y, INT32_MIN, INT32_MAX);
        const width = finiteInteger(rect.width, 1, INT32_MAX);
        const height = finiteInteger(rect.height, 1, INT32_MAX);
        const scale = safeCall(() => this._display.get_monitor_scale(monitor));
        if (
            x === null ||
            y === null ||
            width === null ||
            height === null ||
            typeof scale !== 'number' ||
            !Number.isFinite(scale) ||
            scale <= 0
        ) {
            return null;
        }
        return {
            pid,
            title: boundedTitle(safeCall(() => window.get_title(), '')),
            appId: identity,
            x,
            y,
            width,
            height,
            scale: String(scale),
        };
    }

    _requestReport(window, force, retries = 0) {
        const report = this._normalizedWindow(window);
        if (report === null) {
            if (window === this._gameWindow || window === this._candidateWindow)
                this._requestClear();
            return;
        }
        const signature = JSON.stringify(report);
        if (!force && signature === this._lastReportSignature)
            return;
        this._clearAuthorityFence = false;
        this._queueWindowRequest({
            kind: 'report',
            window,
            retries,
            serial: ++this._windowIntentSerial,
        });
    }

    _requestClear() {
        this._cancelCandidateRetry();
        this._lastReportSignature = null;
        this._clearAuthorityFence = true;
        this._applyPassiveState();
        this._queueWindowRequest({
            kind: 'clear',
            serial: ++this._windowIntentSerial,
        });
    }

    _queueWindowRequest(request) {
        if (this._windowRequestInFlight) {
            this._pendingWindowRequest = request;
            return;
        }
        this._dispatchWindowRequest(request);
    }

    _dispatchWindowRequest(request) {
        this._windowRequestInFlight = true;
        let method = 'ClearWindow';
        let signature = '()';
        let values = [];
        if (request.kind === 'report') {
            const report = this._normalizedWindow(request.window);
            if (report !== null) {
                method = 'ReportWindow';
                signature = '(issiiiis)';
                values = [
                    report.pid,
                    report.title,
                    report.appId,
                    report.x,
                    report.y,
                    report.width,
                    report.height,
                    report.scale,
                ];
                this._lastReportSignature = JSON.stringify(report);
            } else {
                this._lastReportSignature = null;
            }
        }
        const canRetry = method === 'ReportWindow';
        const finish = () =>
            this._requestSnapshot(
                () => {
                    const current =
                        request.serial === this._windowIntentSerial;
                    const accepted =
                        current &&
                        request.kind === 'report' &&
                        this._gameWindow === request.window;
                    this._finishWindowRequest();
                    if (current && canRetry && !accepted)
                        this._scheduleCandidateRetry(request);
                },
                fail,
                request.serial,
            );
        const fail = () => {
            const current = request.serial === this._windowIntentSerial;
            if (current)
                this._applyPassiveState();
            this._finishWindowRequest();
            if (current && canRetry)
                this._scheduleCandidateRetry(request);
        };
        this._callStringMethod(method, signature, values, finish, fail);
    }

    _finishWindowRequest() {
        this._windowRequestInFlight = false;
        const pending = this._pendingWindowRequest;
        this._pendingWindowRequest = null;
        if (pending !== null)
            this._dispatchWindowRequest(pending);
    }

    _scheduleCandidateRetry(request) {
        this._cancelCandidateRetry();
        if (
            request.retries <= 0 ||
            this._coreOwner === null ||
            this._normalizedWindow(request.window) === null
        ) {
            return;
        }
        this._candidateWindow = request.window;
        this._candidateRetrySource = this._addTimeout(
            CANDIDATE_RETRY_MS,
            false,
            () => {
                this._candidateRetrySource = 0;
                const focus = this._display.get_focus_window();
                if (
                    focus !== request.window &&
                    focus !== this._overlayWindow
                ) {
                    return false;
                }
                this._candidateWindow = request.window;
                this._requestReport(
                    request.window,
                    true,
                    request.retries - 1,
                );
                return false;
            },
        );
    }

    _applySnapshotJson(json) {
        if (
            typeof json !== 'string' ||
            json.length === 0 ||
            json.length > MAX_SNAPSHOT_BYTES
        ) {
            this._applyPassiveState();
            return false;
        }
        let payload;
        try {
            payload = JSON.parse(json);
        } catch {
            this._applyPassiveState();
            return false;
        }
        const envelope = this._snapshotFromEnvelope(payload);
        if (envelope === null) {
            this._applyPassiveState();
            return false;
        }
        if (envelope.revision < this._lastSnapshotRevision)
            return false;
        this._lastSnapshotRevision = envelope.revision;
        const snapshot = envelope.snapshot;
        if (
            typeof snapshot !== 'object' ||
            !['Passive', 'Interactive'].includes(snapshot.overlay_mode)
        ) {
            this._applyPassiveState();
            return false;
        }

        const previousMode = this._mode;
        this._mode = snapshot.overlay_mode;
        const active = snapshot.active_game;
        if (this._clearAuthorityFence && active !== null) {
            this._applyPassiveState();
            return false;
        }
        if (active === null) {
            this._applyPassiveState();
            return false;
        }
        const pid = finiteInteger(active?.pid, 1, INT32_MAX);
        const appId = boundedIdentity(active?.app_id);
        const rect = this._snapshotRect(active?.rect);
        if (pid === null || appId === null || rect === null) {
            this._applyPassiveState();
            return false;
        }
        const game = this._findGameWindow(pid, appId, rect);
        if (game === null) {
            this._requestClear();
            return false;
        }
        const focus = this._display.get_focus_window();
        if (
            focus !== null &&
            focus !== this._overlayWindow &&
            focus !== game
        ) {
            this._candidateWindow = focus;
            this._cancelCandidateRetry();
            this._requestReport(focus, true, MAX_CANDIDATE_RETRIES);
            return false;
        }
        this._cancelCandidateRetry();
        this._gameWindow = game;
        this._candidateWindow = game;
        this._placeOverlay();
        this._refreshShortcuts();

        if (
            this._mode === 'Interactive' &&
            this._overlayWindow !== null &&
            (previousMode !== 'Interactive' || focus === this._gameWindow)
        ) {
            this._activate(this._overlayWindow);
        } else if (
            this._mode === 'Passive' &&
            focus === this._overlayWindow
        ) {
            this._activate(this._gameWindow);
        }
        return true;
    }

    _snapshotFromEnvelope(value) {
        if (
            value === null ||
            typeof value !== 'object' ||
            Array.isArray(value) ||
            !Number.isSafeInteger(value.revision) ||
            value.revision < 0 ||
            value.snapshot === null ||
            typeof value.snapshot !== 'object' ||
            Array.isArray(value.snapshot)
        ) {
            return null;
        }
        return {
            revision: value.revision,
            snapshot: value.snapshot,
        };
    }

    _applyPassiveState() {
        this._mode = 'Passive';
        this._candidateWindow = null;
        this._gameWindow = null;
        this._releaseOverlayAuthority();
        this._releaseShortcuts();
        this._reportShortcutAvailability('disabled');
    }

    _snapshotRect(value) {
        if (value === null || typeof value !== 'object' || Array.isArray(value))
            return null;
        const rect = {
            x: finiteInteger(value.x, INT32_MIN, INT32_MAX),
            y: finiteInteger(value.y, INT32_MIN, INT32_MAX),
            width: finiteInteger(value.width, 1, INT32_MAX),
            height: finiteInteger(value.height, 1, INT32_MAX),
        };
        return Object.values(rect).includes(null) ? null : rect;
    }

    _findGameWindow(pid, appId, activeRect) {
        const candidates = [];
        for (const window of this._windowSignals.keys()) {
            const normalized = this._normalizedWindow(window);
            if (
                normalized !== null &&
                normalized.pid === pid &&
                normalized.appId === appId
            ) {
                candidates.push({window, normalized});
            }
        }
        const focus = this._display.get_focus_window();
        const focused = candidates.find(candidate => candidate.window === focus);
        if (focused !== undefined)
            return focused.window;
        const exact = candidates.filter(candidate =>
            sameRect(candidate.normalized, activeRect),
        );
        if (exact.length === 1)
            return exact[0].window;
        return candidates.length === 1 ? candidates[0].window : null;
    }

    _placeOverlay() {
        if (
            this._placingOverlay ||
            this._overlayWindow === null ||
            this._gameWindow === null
        ) {
            return;
        }
        const game = this._normalizedWindow(this._gameWindow);
        if (game === null) {
            this._requestClear();
            return;
        }
        const overlay = this._overlayWindow;
        this._placingOverlay = true;
        try {
            const gameMonitor = this._gameWindow.get_monitor();
            if (safeCall(() => overlay.get_monitor()) !== gameMonitor)
                safeCall(() => overlay.move_to_monitor(gameMonitor));
            const workspace = safeCall(() => this._gameWindow.get_workspace());
            if (
                workspace !== null &&
                safeCall(() => overlay.get_workspace()) !== workspace
            ) {
                safeCall(() => overlay.change_workspace(workspace));
            }
            const target = {
                x: game.x,
                y: game.y,
                width: game.width,
                height: game.height,
            };
            const current = safeCall(() => overlay.get_frame_rect());
            if (!sameRect(current, target)) {
                safeCall(() =>
                    overlay.move_resize_frame(
                        false,
                        target.x,
                        target.y,
                        target.width,
                        target.height,
                    ),
                );
            }
            safeCall(() => overlay.make_above());
            safeCall(() => overlay.raise());
        } finally {
            this._placingOverlay = false;
        }
    }

    _releaseOverlayAuthority() {
        if (this._overlayWindow !== null)
            safeCall(() => this._overlayWindow.unmake_above());
    }

    _activate(window) {
        if (window !== null) {
            const timestamp = safeCall(() => this._display.get_current_time(), 0);
            safeCall(() => window.activate(timestamp));
        }
    }

    _refreshShortcuts() {
        const serial = ++this._shortcutRequestSerial;
        this._callStringMethod(
            'GnomeShortcutDefinitions',
            '()',
            [],
            json => {
                if (serial !== this._shortcutRequestSerial)
                    return;
                const definitions = parseShortcutDefinitions(json);
                if (definitions === null) {
                    this._releaseShortcuts();
                    this._reportShortcutAvailability('conflict');
                    return;
                }
                this._applyShortcuts(definitions);
            },
            () => {
                if (serial === this._shortcutRequestSerial) {
                    this._releaseShortcuts();
                    this._reportShortcutAvailability('conflict');
                }
            },
        );
    }

    _applyShortcuts(definitions) {
        const signature = JSON.stringify(definitions);
        if (signature === this._shortcutSignature) {
            this._reportShortcutAvailability(
                definitions.length === 0 ? 'disabled' : 'available',
            );
            return;
        }
        this._releaseShortcuts();
        if (definitions.length === 0) {
            this._shortcutSignature = signature;
            this._reportShortcutAvailability('disabled');
            return;
        }

        for (const definition of definitions) {
            const actionId = safeCall(
                () =>
                    this._display.grab_accelerator(
                        definition.accelerator,
                        this._Meta.KeyBindingFlags.NONE,
                    ),
                this._Meta.KeyBindingAction.NONE,
            );
            if (
                !Number.isInteger(actionId) ||
                actionId <= this._Meta.KeyBindingAction.NONE ||
                !this._setShortcutActionMode(
                    actionId,
                    Shell.ActionMode.NORMAL,
                )
            ) {
                if (
                    Number.isInteger(actionId) &&
                    actionId > this._Meta.KeyBindingAction.NONE
                ) {
                    safeCall(() => this._display.ungrab_accelerator(actionId));
                }
                this._releaseShortcuts();
                this._reportShortcutAvailability('conflict');
                return;
            }
            this._actionToShortcut.set(actionId, definition.id);
        }
        this._shortcutSignature = signature;
        this._reportShortcutAvailability('available');
    }

    _releaseShortcuts() {
        for (const actionId of this._actionToShortcut.keys()) {
            this._setShortcutActionMode(actionId, Shell.ActionMode.NONE);
            safeCall(() => this._display.ungrab_accelerator(actionId));
        }
        this._actionToShortcut.clear();
        this._shortcutSignature = null;
    }

    _setShortcutActionMode(actionId, mode) {
        const bindingName = boundedIdentity(
            safeCall(() =>
                this._Meta.external_binding_name_for_action(actionId),
            ),
        );
        if (bindingName === null)
            return false;
        return safeCall(() => {
            Main.wm.allowKeybinding(bindingName, mode);
            return true;
        }, false);
    }

    _onAccelerator(actionId) {
        const shortcutId = this._actionToShortcut.get(actionId);
        const method = SHORTCUT_METHODS.get(shortcutId);
        if (method !== undefined)
            this._callStringMethod(method, '()', [], () =>
                this._requestSnapshot(),
            );
    }

    _requestSnapshot(
        callback = null,
        errorCallback = null,
        intentSerial = this._windowIntentSerial,
    ) {
        this._callStringMethod(
            'SnapshotVersioned',
            '()',
            [],
            json => {
                if (intentSerial === this._windowIntentSerial)
                    this._applySnapshotJson(json);
                callback?.();
            },
            errorCallback,
        );
    }

    _reportShortcutAvailability(state) {
        if (
            this._coreOwner === null ||
            state === this._reportedAvailability
        ) {
            return;
        }
        this._reportedAvailability = state;
        this._callCore(
            'ReportGnomeShortcutAvailability',
            '(s)',
            [state],
            '()',
            null,
            () => {
                if (this._reportedAvailability === state)
                    this._reportedAvailability = null;
            },
        );
    }

    _callStringMethod(
        method,
        signature,
        values,
        callback,
        errorCallback = null,
    ) {
        this._callCore(
            method,
            signature,
            values,
            '(s)',
            reply => {
                const [value] = safeCall(() => reply.deepUnpack(), []);
                if (typeof value === 'string')
                    callback(value);
                else
                    errorCallback?.();
            },
            errorCallback,
        );
    }

    _callCore(
        method,
        signature,
        values,
        replySignature,
        callback = null,
        errorCallback = null,
    ) {
        if (this._coreOwner === null)
            return;
        const generation = this._generation;
        const parameters = new this._GLib.Variant(signature, values);
        const replyType = new this._GLib.VariantType(replySignature);
        try {
            this._dbus.call(
                CORE_BUS_NAME,
                CORE_PATH,
                CORE_INTERFACE,
                method,
                parameters,
                replyType,
                this._Gio.DBusCallFlags.NONE,
                DBUS_TIMEOUT_MS,
                null,
                (connection, result) => {
                    if (
                        generation !== this._generation ||
                        this._coreOwner === null
                    ) {
                        return;
                    }
                    try {
                        const reply = connection.call_finish(result);
                        callback?.(reply);
                    } catch {
                        errorCallback?.();
                    }
                },
            );
        } catch {
            errorCallback?.();
        }
    }

    _addTimeout(milliseconds, repeat, callback) {
        let sourceId = 0;
        sourceId = this._GLib.timeout_add(
            this._GLib.PRIORITY_DEFAULT,
            milliseconds,
            () => {
                if (this._coreOwner === null) {
                    this._sources.delete(sourceId);
                    return this._GLib.SOURCE_REMOVE;
                }
                const callbackResult = callback();
                const keep = repeat && callbackResult === true;
                if (!keep)
                    this._sources.delete(sourceId);
                return keep
                    ? this._GLib.SOURCE_CONTINUE
                    : this._GLib.SOURCE_REMOVE;
            },
        );
        this._sources.add(sourceId);
        return sourceId;
    }

    _removeSource(sourceId) {
        if (sourceId !== 0 && this._sources.delete(sourceId))
            safeCall(() => this._GLib.source_remove(sourceId));
    }

    _cancelFocusGrace() {
        if (this._focusGraceSource !== 0) {
            this._removeSource(this._focusGraceSource);
            this._focusGraceSource = 0;
        }
    }

    _cancelCandidateRetry() {
        if (this._candidateRetrySource !== 0) {
            this._removeSource(this._candidateRetrySource);
            this._candidateRetrySource = 0;
        }
    }
}

export default class OverCrowExtension extends Extension {
    enable() {
        this._runtime = null;
        if (!safeCall(() => Meta.is_wayland_compositor(), false))
            return;
        this._runtime = new GnomeBridgeRuntime({
            Gio,
            GLib,
            Meta,
            display: global.display,
            dbus: Gio.DBus.session,
        });
        this._runtime.start();
    }

    disable() {
        this._runtime?.stop();
        this._runtime = null;
    }
}
