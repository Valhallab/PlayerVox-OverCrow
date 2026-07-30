import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const projectRoot = new URL('..', import.meta.url);
const metadataPath = new URL('integrations/gnome/metadata.json', projectRoot);
const extensionPath = new URL('integrations/gnome/extension.js', projectRoot);

class SignalEmitter {
    constructor() {
        this.nextSignalId = 1;
        this.signals = new Map();
    }

    connect(name, callback) {
        const id = this.nextSignalId++;
        this.signals.set(id, {name, callback});
        return id;
    }

    disconnect(id) {
        this.signals.delete(id);
    }

    emit(name, ...args) {
        for (const signal of this.signals.values()) {
            if (signal.name === name)
                signal.callback(this, ...args);
        }
    }
}

class FakeWindow extends SignalEmitter {
    constructor({pid, appId, rect, monitor = 0, workspace = {id: 1}}) {
        super();
        this.pid = pid;
        this.appId = appId;
        this.rect = {...rect};
        this.monitor = monitor;
        this.workspace = workspace;
        this.minimized = false;
        this.hidden = false;
        this.above = false;
        this.activated = 0;
        this.raised = 0;
        this.moves = [];
    }

    get_pid() {
        return this.pid;
    }

    get_sandboxed_app_id() {
        return null;
    }

    get_gtk_application_id() {
        return this.appId;
    }

    get_wm_class() {
        return this.appId;
    }

    get_title() {
        return `${this.appId} title`;
    }

    get_frame_rect() {
        return {...this.rect};
    }

    get_monitor() {
        return this.monitor;
    }

    get_workspace() {
        return this.workspace;
    }

    is_hidden() {
        return this.hidden;
    }

    is_override_redirect() {
        return false;
    }

    move_to_monitor(monitor) {
        this.monitor = monitor;
    }

    change_workspace(workspace) {
        this.workspace = workspace;
    }

    move_resize_frame(_userOperation, x, y, width, height) {
        this.rect = {x, y, width, height};
        this.moves.push({...this.rect});
    }

    make_above() {
        this.above = true;
    }

    unmake_above() {
        this.above = false;
    }

    raise() {
        this.raised += 1;
    }

    activate() {
        this.activated += 1;
    }
}

class FakeDisplay extends SignalEmitter {
    constructor(windows) {
        super();
        this.windows = windows;
        this.focusWindow = null;
        this.nextActionId = 1;
        this.grabs = new Map();
        this.released = [];
    }

    list_all_windows() {
        return [...this.windows];
    }

    get_focus_window() {
        return this.focusWindow;
    }

    get_monitor_scale(monitor) {
        return monitor === 1 ? 2 : 1;
    }

    get_current_time() {
        return 42;
    }

    grab_accelerator(accelerator) {
        if (accelerator.includes('Conflict'))
            return 0;
        const actionId = this.nextActionId++;
        this.grabs.set(actionId, accelerator);
        return actionId;
    }

    ungrab_accelerator(actionId) {
        this.released.push(actionId);
        return this.grabs.delete(actionId);
    }

    focus(window) {
        this.focusWindow = window;
        this.emit('notify::focus-window');
    }
}

class Variant {
    constructor(_signature, value) {
        this.value = value;
    }

    deepUnpack() {
        return this.value;
    }
}

function passiveSnapshot(activeGame = null) {
    return {
        active_game: activeGame,
        overlay_mode: 'Passive',
        manual_stopwatch: {elapsed_ms: 0, running: false},
    };
}

class FakeDbus {
    constructor() {
        this.calls = [];
        this.snapshot = passiveSnapshot();
        this.signalSubscriptions = new Map();
        this.nextSubscriptionId = 1;
        this.availability = [];
        this.deferredMethods = new Set();
        this.pendingCalls = [];
        this.failedMethods = new Set();
        this.rejectedReports = 0;
        this.revision = 0;
    }

    signal_subscribe(
        _sender,
        _interface,
        _member,
        _path,
        _argument,
        _flags,
        callback,
    ) {
        const id = this.nextSubscriptionId++;
        this.signalSubscriptions.set(id, callback);
        return id;
    }

    signal_unsubscribe(id) {
        this.signalSubscriptions.delete(id);
    }

    call(
        _bus,
        _path,
        _interface,
        method,
        parameters,
        _replyType,
        _flags,
        _timeout,
        _cancellable,
        callback,
    ) {
        const args = parameters?.deepUnpack() ?? [];
        this.calls.push({method, args});
        if (this.failedMethods.delete(method)) {
            callback?.(this, {error: true});
            return;
        }
        if (this.deferredMethods.has(method)) {
            this.pendingCalls.push({method, args, callback});
            return;
        }
        this._complete(method, args, callback);
    }

    _complete(method, args, callback) {
        let reply = [];
        if (method === 'Snapshot') {
            reply = [JSON.stringify(this.snapshot)];
        } else if (method === 'SnapshotVersioned') {
            reply = [
                JSON.stringify({
                    revision: this.revision,
                    snapshot: this.snapshot,
                }),
            ];
        } else if (method === 'ReportWindow') {
            const previous = JSON.stringify(this.snapshot);
            const [pid, title, appId, x, y, width, height, scale] = args;
            if (this.rejectedReports > 0) {
                this.rejectedReports -= 1;
                this.snapshot = passiveSnapshot();
            } else {
                this.snapshot =
                    pid === 42
                        ? passiveSnapshot({
                              pid,
                              steam_app_id: 620,
                              app_id: appId,
                              title,
                              rect: {x, y, width, height},
                              scale: Number(scale),
                              backend: 'wayland',
                          })
                        : passiveSnapshot();
            }
            if (JSON.stringify(this.snapshot) !== previous)
                this.revision += 1;
            reply = [JSON.stringify(this.snapshot)];
        } else if (method === 'ClearWindow') {
            const previous = JSON.stringify(this.snapshot);
            this.snapshot = passiveSnapshot();
            if (JSON.stringify(this.snapshot) !== previous)
                this.revision += 1;
            reply = [JSON.stringify(this.snapshot)];
        } else if (method === 'GnomeShortcutDefinitions') {
            reply = [
                JSON.stringify(
                    this.snapshot.active_game
                        ? [{id: 'toggle-overlay', accelerator: 'Meta+Alt+O'}]
                        : [],
                ),
            ];
        } else if (method === 'ReportGnomeShortcutAvailability') {
            this.availability.push(args[0]);
        } else if (method === 'ToggleOverlay') {
            this.snapshot.overlay_mode =
                this.snapshot.overlay_mode === 'Passive'
                    ? 'Interactive'
                    : 'Passive';
            this.revision += 1;
            reply = [JSON.stringify(this.snapshot)];
        } else if (
            method === 'ToggleManualStopwatch' ||
            method === 'ResetManualStopwatch'
        ) {
            reply = [JSON.stringify(this.snapshot)];
        }
        callback?.(this, {reply: new Variant('', reply)});
    }

    completeNext(method) {
        const index = this.pendingCalls.findIndex(call => call.method === method);
        assert.notEqual(index, -1, `no pending ${method} call`);
        const [pending] = this.pendingCalls.splice(index, 1);
        this._complete(pending.method, pending.args, pending.callback);
    }

    completeNextWithString(method, value) {
        const index = this.pendingCalls.findIndex(call => call.method === method);
        assert.notEqual(index, -1, `no pending ${method} call`);
        const [pending] = this.pendingCalls.splice(index, 1);
        pending.callback?.(this, {reply: new Variant('', [value])});
    }

    call_finish(result) {
        if (result.error)
            throw new Error('forced D-Bus failure');
        return result.reply;
    }

    emitSnapshot(snapshot) {
        this.snapshot = snapshot;
        this.revision += 1;
        const parameters = new Variant('', [
            JSON.stringify({revision: this.revision, snapshot}),
        ]);
        for (const callback of this.signalSubscriptions.values())
            callback(this, ':1.42', '', '', '', parameters);
    }
}

function createHarness(windows) {
    const display = new FakeDisplay(windows);
    const dbus = new FakeDbus();
    const sources = new Map();
    let nextSourceId = 1;
    let nameWatch = null;
    const GLib = {
        PRIORITY_DEFAULT: 0,
        SOURCE_CONTINUE: true,
        SOURCE_REMOVE: false,
        Variant,
        VariantType: class {
            constructor(signature) {
                this.signature = signature;
            }
        },
        timeout_add(_priority, _milliseconds, callback) {
            const id = nextSourceId++;
            sources.set(id, callback);
            return id;
        },
        source_remove(id) {
            return sources.delete(id);
        },
    };
    const Gio = {
        BusType: {SESSION: 0},
        BusNameWatcherFlags: {NONE: 0},
        DBusCallFlags: {NONE: 0},
        DBusSignalFlags: {NONE: 0},
        DBus: {session: dbus},
        bus_watch_name(_type, _name, _flags, appeared, vanished) {
            nameWatch = {appeared, vanished};
            return 1;
        },
        bus_unwatch_name() {
            nameWatch = null;
        },
    };
    const Meta = {
        KeyBindingFlags: {NONE: 0},
        KeyBindingAction: {NONE: 0},
        external_binding_name_for_action: actionId =>
            `external-grab-${actionId}`,
        is_wayland_compositor: () => true,
    };
    const Shell = {
        ActionMode: {
            NONE: 0,
            NORMAL: 1,
        },
    };
    const allowedBindings = new Map();
    const Main = {
        wm: {
            allowKeybinding(name, mode) {
                allowedBindings.set(name, mode);
            },
        },
    };
    class Extension {}
    const context = {
        Gio,
        GLib,
        Main,
        Meta,
        Shell,
        Extension,
        global: {display},
        console: {error() {}},
    };
    let source = readFileSync(extensionPath, 'utf8')
        .replace(/^import .*;\n/gm, '')
        .replace('export class GnomeBridgeRuntime', 'class GnomeBridgeRuntime')
        .replace(
            'export default class OverCrowExtension',
            'class OverCrowExtension',
        );
    source +=
        '\nglobalThis.__overcrowExports = {GnomeBridgeRuntime, OverCrowExtension};\n';
    vm.runInNewContext(source, context, {filename: extensionPath.pathname});

    return {
        Runtime: context.__overcrowExports.GnomeBridgeRuntime,
        Extension: context.__overcrowExports.OverCrowExtension,
        display,
        dbus,
        Gio,
        GLib,
        Meta,
        Shell,
        allowedBindings,
        sources,
        hasNameWatch() {
            return nameWatch !== null;
        },
        coreAppeared() {
            nameWatch.appeared(dbus, 'io.github.overcrow.Core1', ':1.42');
        },
        coreVanished() {
            nameWatch.vanished(dbus, 'io.github.overcrow.Core1');
        },
        runSourcesOnce() {
            for (const [id, callback] of [...sources]) {
                if (callback() === GLib.SOURCE_REMOVE)
                    sources.delete(id);
            }
        },
    };
}

test('metadata declares the exact GNOME 46 through 50 support window', () => {
    const metadata = JSON.parse(readFileSync(metadataPath, 'utf8'));
    assert.equal(metadata.uuid, 'overcrow@playervox.com');
    assert.deepEqual(metadata['shell-version'], ['46', '47', '48', '49', '50']);
    assert.deepEqual(metadata['session-modes'], ['user', 'ubuntu']);
});

test('Core absence leaves the compositor and shortcuts untouched', () => {
    const harness = createHarness([]);
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });

    runtime.start();

    assert.equal(harness.display.signals.size, 0);
    assert.equal(harness.display.grabs.size, 0);
    assert.equal(harness.sources.size, 0);
    runtime.stop();
});

test('the extension remains inert when GNOME runs on X11', () => {
    const harness = createHarness([]);
    harness.Meta.is_wayland_compositor = () => false;
    const extension = new harness.Extension();

    extension.enable();

    assert.equal(harness.hasNameWatch(), false);
    assert.equal(harness.display.signals.size, 0);
    assert.equal(harness.sources.size, 0);
    extension.disable();
});

test('a selected game places the passive overlay without taking focus', () => {
    const workspace = {id: 7};
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 100, y: 50, width: 1280, height: 720},
        monitor: 1,
        workspace,
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 0, y: 0, width: 400, height: 300},
    });
    const harness = createHarness([game, overlay]);
    harness.display.focusWindow = game;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });

    runtime.start();
    harness.coreAppeared();

    assert.deepEqual(overlay.rect, game.rect);
    assert.equal(overlay.monitor, game.monitor);
    assert.equal(overlay.workspace, workspace);
    assert.equal(overlay.above, true);
    assert.equal(overlay.activated, 0);
    assert.deepEqual([...harness.display.grabs.values()], ['<Super><Alt>O']);
    assert.equal(
        harness.allowedBindings.get('external-grab-1'),
        harness.Shell.ActionMode.NORMAL,
    );
    assert.equal(harness.dbus.availability.at(-1), 'available');
});

test('a failed shortcut availability report is retried on refresh', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 0, y: 0, width: 800, height: 600},
    });
    const harness = createHarness([game]);
    harness.display.focusWindow = game;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });
    runtime.start();
    harness.coreAppeared();
    const reportsBefore = harness.dbus.calls.filter(
        call => call.method === 'ReportGnomeShortcutAvailability',
    ).length;

    runtime._reportedAvailability = null;
    harness.dbus.failedMethods.add('ReportGnomeShortcutAvailability');
    runtime._reportShortcutAvailability('available');
    runtime._refreshShortcuts();

    assert.equal(
        harness.dbus.calls.filter(
            call => call.method === 'ReportGnomeShortcutAvailability',
        ).length,
        reportsBefore + 2,
    );
    assert.equal(harness.dbus.availability.at(-1), 'available');
});

test('a focused game window wins over another window from the same process', () => {
    const secondary = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 40, y: 50, width: 480, height: 320},
    });
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 100, y: 80, width: 1280, height: 720},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 0, y: 0, width: 400, height: 300},
    });
    const harness = createHarness([secondary, game, overlay]);
    harness.display.focusWindow = game;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });

    runtime.start();
    harness.coreAppeared();

    assert.deepEqual(overlay.rect, game.rect);
});

test('an older baseline snapshot cannot overwrite a newer signal', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 100, y: 80, width: 1280, height: 720},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 0, y: 0, width: 400, height: 300},
    });
    const harness = createHarness([game, overlay]);
    harness.dbus.deferredMethods.add('SnapshotVersioned');
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });

    runtime.start();
    harness.coreAppeared();
    harness.dbus.emitSnapshot(
        passiveSnapshot({
            pid: game.pid,
            steam_app_id: 620,
            app_id: game.appId,
            title: game.title,
            rect: game.rect,
            scale: 1,
            backend: 'wayland',
        }),
    );
    harness.dbus.completeNextWithString(
        'SnapshotVersioned',
        JSON.stringify({revision: 0, snapshot: passiveSnapshot()}),
    );

    assert.equal(runtime._gameWindow, game);
    assert.deepEqual(overlay.rect, game.rect);
    assert.equal(overlay.above, true);
});

test('an equal revision rebinds an equivalent replacement game window', () => {
    const original = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 100, y: 80, width: 1280, height: 720},
    });
    const replacement = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 100, y: 80, width: 1280, height: 720},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 0, y: 0, width: 400, height: 300},
    });
    const harness = createHarness([original, replacement, overlay]);
    harness.display.focusWindow = original;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });

    runtime.start();
    harness.coreAppeared();
    const revision = harness.dbus.revision;
    harness.display.focus(replacement);

    assert.equal(harness.dbus.revision, revision);
    assert.equal(runtime._gameWindow, replacement);
    assert.equal(overlay.above, true);
});

test('a newly started focused game is retried while its overlay maps', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 100, y: 80, width: 1280, height: 720},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 0, y: 0, width: 400, height: 300},
    });
    const harness = createHarness([game]);
    harness.display.focusWindow = game;
    harness.dbus.rejectedReports = 1;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });

    runtime.start();
    harness.coreAppeared();
    assert.equal(harness.dbus.snapshot.active_game, null);

    harness.display.windows.push(overlay);
    harness.display.emit('window-created', overlay);
    harness.display.focus(overlay);
    harness.runSourcesOnce();

    assert.equal(harness.dbus.snapshot.active_game.pid, game.pid);
    assert.equal(runtime._gameWindow, game);
    assert.deepEqual(overlay.rect, game.rect);
    assert.deepEqual([...harness.display.grabs.values()], ['<Super><Alt>O']);
});

test('a newly created focused game is reported after its identity settles', () => {
    const game = new FakeWindow({
        pid: null,
        appId: null,
        rect: {x: 100, y: 80, width: 1280, height: 720},
    });
    const harness = createHarness([]);
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });

    runtime.start();
    harness.coreAppeared();
    harness.display.windows.push(game);
    harness.display.focus(game);
    harness.display.emit('window-created', game);
    assert.equal(harness.dbus.snapshot.active_game, null);

    game.appId = 'steam_app_620';
    game.emit('notify::wm-class');
    assert.equal(harness.dbus.snapshot.active_game, null);

    game.pid = 42;
    game.emit('shown');

    assert.equal(harness.dbus.snapshot.active_game.pid, game.pid);
    assert.equal(runtime._gameWindow, game);
});

test('a rejected focused candidate has a bounded retry budget', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 100, y: 80, width: 1280, height: 720},
    });
    const harness = createHarness([game]);
    harness.display.focusWindow = game;
    harness.dbus.rejectedReports = 100;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });

    runtime.start();
    harness.coreAppeared();
    for (let attempt = 0; attempt < 20; attempt += 1)
        harness.runSourcesOnce();

    assert.equal(
        harness.dbus.calls.filter(call => call.method === 'ReportWindow').length,
        7,
    );
    assert.equal(harness.dbus.snapshot.active_game, null);
});

test('a keepalive cannot overwrite a newer unrelated focus report', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 10, y: 20, width: 800, height: 600},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 10, y: 20, width: 800, height: 600},
    });
    const editor = new FakeWindow({
        pid: 99,
        appId: 'org.gnome.TextEditor',
        rect: {x: 900, y: 20, width: 600, height: 600},
    });
    const harness = createHarness([game, overlay, editor]);
    harness.display.focusWindow = game;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });
    runtime.start();
    harness.coreAppeared();
    harness.dbus.deferredMethods.add('ReportWindow');

    harness.runSourcesOnce();
    harness.display.focus(editor);
    assert.equal(overlay.above, false);
    harness.dbus.emitSnapshot(harness.dbus.snapshot);
    assert.equal(runtime._gameWindow, null);
    harness.runSourcesOnce();
    harness.dbus.completeNext('ReportWindow');
    harness.dbus.completeNext('ReportWindow');

    const reports = harness.dbus.calls.filter(
        call => call.method === 'ReportWindow',
    );
    assert.equal(reports.at(-1).args[0], 99);
    assert.equal(harness.dbus.snapshot.active_game, null);
});

test('a stale keepalive snapshot cannot overwrite a newer clear intent', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 10, y: 20, width: 800, height: 600},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 10, y: 20, width: 800, height: 600},
    });
    const harness = createHarness([game, overlay]);
    harness.display.focusWindow = game;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });
    runtime.start();
    harness.coreAppeared();
    const activeSnapshot = harness.dbus.snapshot;
    const staleSnapshot = JSON.stringify({
        revision: harness.dbus.revision,
        snapshot: activeSnapshot,
    });
    harness.dbus.deferredMethods.add('SnapshotVersioned');

    harness.runSourcesOnce();
    harness.display.focus(null);
    harness.runSourcesOnce();
    assert.equal(runtime._gameWindow, null);
    assert.equal(overlay.above, false);

    harness.dbus.emitSnapshot(passiveSnapshot());
    harness.dbus.emitSnapshot(activeSnapshot);
    assert.equal(runtime._gameWindow, null);
    assert.equal(overlay.above, false);

    harness.dbus.completeNextWithString('SnapshotVersioned', staleSnapshot);

    assert.equal(runtime._gameWindow, null);
    assert.equal(overlay.above, false);
});

test('interactive focus is retained while unrelated application focus is respected', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 10, y: 20, width: 800, height: 600},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 0, y: 0, width: 400, height: 300},
    });
    const editor = new FakeWindow({
        pid: 99,
        appId: 'org.gnome.TextEditor',
        rect: {x: 900, y: 20, width: 600, height: 600},
    });
    const harness = createHarness([game, overlay, editor]);
    harness.display.focusWindow = game;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });
    runtime.start();
    harness.coreAppeared();

    harness.dbus.emitSnapshot({...harness.dbus.snapshot, overlay_mode: 'Interactive'});
    assert.equal(overlay.activated, 1);

    harness.display.focus(game);
    assert.equal(overlay.activated, 2);

    harness.display.focus(editor);
    assert.equal(overlay.activated, 2);
    assert.equal(harness.dbus.snapshot.active_game, null);
});

test('an impossible interactive snapshot without a game fails closed to Passive', () => {
    const harness = createHarness([]);
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });
    runtime.start();
    harness.coreAppeared();

    harness.dbus.emitSnapshot({
        ...passiveSnapshot(),
        overlay_mode: 'Interactive',
    });

    assert.equal(runtime._mode, 'Passive');
    assert.equal(harness.display.grabs.size, 0);
});

test('shortcut activation dispatches only known Core methods', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 0, y: 0, width: 800, height: 600},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 0, y: 0, width: 800, height: 600},
    });
    const harness = createHarness([game, overlay]);
    harness.display.focusWindow = game;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });
    runtime.start();
    harness.coreAppeared();
    const [actionId] = harness.display.grabs.keys();

    harness.display.emit('accelerator-activated', actionId);
    harness.display.emit('accelerator-activated', 9999);

    assert.equal(
        harness.dbus.calls.filter(call => call.method === 'ToggleOverlay').length,
        1,
    );
});

test('a settled null focus clears stale game authority after the grace period', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 0, y: 0, width: 800, height: 600},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 0, y: 0, width: 800, height: 600},
    });
    const harness = createHarness([game, overlay]);
    harness.display.focusWindow = game;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });
    runtime.start();
    harness.coreAppeared();
    const clearsBefore = harness.dbus.calls.filter(
        call => call.method === 'ClearWindow',
    ).length;

    harness.display.focus(null);
    harness.runSourcesOnce();

    assert.equal(
        harness.dbus.calls.filter(call => call.method === 'ClearWindow').length,
        clearsBefore + 1,
    );
});

test('Core loss and disable release every owned resource and above state', () => {
    const game = new FakeWindow({
        pid: 42,
        appId: 'steam_app_620',
        rect: {x: 0, y: 0, width: 800, height: 600},
    });
    const overlay = new FakeWindow({
        pid: 77,
        appId: 'io.github.overcrow.Overlay',
        rect: {x: 0, y: 0, width: 800, height: 600},
    });
    const harness = createHarness([game, overlay]);
    harness.display.focusWindow = game;
    const runtime = new harness.Runtime({
        Gio: harness.Gio,
        GLib: harness.GLib,
        Meta: harness.Meta,
        display: harness.display,
        dbus: harness.dbus,
    });
    runtime.start();
    harness.coreAppeared();
    assert.notEqual(harness.display.signals.size, 0);
    assert.notEqual(harness.sources.size, 0);

    harness.coreVanished();
    assert.equal(harness.display.signals.size, 0);
    assert.equal(harness.display.grabs.size, 0);
    assert.equal(
        harness.allowedBindings.get('external-grab-1'),
        harness.Shell.ActionMode.NONE,
    );
    assert.equal(harness.sources.size, 0);
    assert.equal(overlay.above, false);

    runtime.stop();
    assert.equal(harness.dbus.signalSubscriptions.size, 0);
});
