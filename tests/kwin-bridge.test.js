"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class Signal {
    constructor() {
        this.handlers = [];
    }

    connect(handler) {
        this.handlers.push(handler);
    }

    emit(...args) {
        for (const handler of this.handlers) {
            handler(...args);
        }
    }
}

function window(overrides = {}) {
    return {
        pid: 42,
        resourceClass: "portal2",
        caption: "Portal 2",
        managed: true,
        deleted: false,
        minimized: false,
        hidden: false,
        frameGeometry: { x: -100, y: 24, width: 1920, height: 1080 },
        output: { devicePixelRatio: 1.25 },
        frameGeometryChanged: new Signal(),
        captionChanged: new Signal(),
        windowClassChanged: new Signal(),
        outputChanged: new Signal(),
        minimizedChanged: new Signal(),
        windowHidden: new Signal(),
        windowShown: new Signal(),
        closed: new Signal(),
        keepAbove: false,
        skipTaskbar: false,
        skipPager: false,
        skipSwitcher: false,
        noBorder: false,
        ...overrides,
    };
}

const game = window();
const overlay = window({
    pid: 84,
    resourceClass: "io.github.overcrow.Overlay",
    caption: "OverCrow",
    frameGeometry: { x: 500, y: 500, width: 320, height: 200 },
});
const calls = [];
const timers = [];
const workspace = {
    activeWindow: game,
    stackingOrder: [game, overlay],
    windowAdded: new Signal(),
    windowRemoved: new Signal(),
    windowActivated: new Signal(),
};

function QTimer() {
    this.interval = 0;
    this.timeout = new Signal();
    this.started = false;
    this.start = () => {
        this.started = true;
    };
    timers.push(this);
}

const source = fs.readFileSync("integrations/kwin/contents/code/main.js", "utf8");
assert.doesNotMatch(
    source,
    /registerShortcut|OverCrowToggleOverlay|Meta\+Alt\+O|ToggleOverlay/,
    "the KWin observer must not retain a permanent shortcut or toggle callback",
);
vm.runInNewContext(source, {
    workspace,
    QTimer,
    callDBus: (...args) => calls.push(args),
    console,
});

const reportCalls = () => calls.filter((call) => call[3] === "ReportWindow");
const clearCalls = () => calls.filter((call) => call[3] === "ClearWindow");
const geometryOf = (target) => {
    const geometry = target.frameGeometry;
    return [geometry.x, geometry.y, geometry.width, geometry.height];
};

assert.deepEqual(
    reportCalls()[0].slice(0, 12),
    [
        "io.github.overcrow.Core1",
        "/io/github/overcrow/Core1",
        "io.github.overcrow.Core1",
        "ReportWindow",
        42,
        "Portal 2",
        "portal2",
        -100,
        24,
        1920,
        1080,
        "1.25",
    ],
    "the initial active window must use the signed-integer Task 4 transport",
);
for (const property of ["keepAbove", "skipTaskbar", "skipPager", "skipSwitcher", "noBorder"]) {
    assert.equal(overlay[property], true, `initial overlay ${property}`);
}
assert.deepEqual(
    geometryOf(overlay),
    [-100, 24, 1920, 1080],
    "an overlay created before the game report must adopt the initial game geometry",
);

const secondOverlay = window({
    pid: 85,
    resourceClass: "io.github.overcrow.Overlay",
    frameGeometry: { x: 600, y: 600, width: 200, height: 100 },
});
workspace.windowAdded.emit(secondOverlay);
for (const property of ["keepAbove", "skipTaskbar", "skipPager", "skipSwitcher", "noBorder"]) {
    assert.equal(secondOverlay[property], true, `new overlay ${property}`);
}
assert.deepEqual(
    geometryOf(secondOverlay),
    [-100, 24, 1920, 1080],
    "an overlay created after the game report must immediately adopt its geometry",
);

const reportsBeforeOverlayFocus = reportCalls().length;
workspace.activeWindow = overlay;
workspace.windowActivated.emit(overlay);
assert.equal(clearCalls().length, 0, "overlay focus must preserve the previous game");
assert.equal(reportCalls().length, reportsBeforeOverlayFocus + 1);
assert.equal(reportCalls().at(-1)[6], "portal2", "the overlay itself is never reported as a game");
overlay.frameGeometry = { x: 700, y: 700, width: 10, height: 10 };
secondOverlay.frameGeometry = { x: 800, y: 800, width: 20, height: 20 };
timers[0].timeout.emit();
assert.deepEqual(geometryOf(overlay), [-100, 24, 1920, 1080], "keepalive repositions the focused overlay");
assert.deepEqual(
    geometryOf(secondOverlay),
    [-100, 24, 1920, 1080],
    "keepalive repositions every live overlay during prolonged overlay focus",
);

game.frameGeometry = { x: 10, y: 20, width: 1280, height: 720 };
game.frameGeometryChanged.emit();
assert.deepEqual(reportCalls().at(-1).slice(7, 11), [10, 20, 1280, 720]);
assert.deepEqual(geometryOf(overlay), [10, 20, 1280, 720]);
assert.deepEqual(geometryOf(secondOverlay), [10, 20, 1280, 720]);

const clearsBeforeTrackedInvalidation = clearCalls().length;
game.frameGeometry = { x: 10, y: 20, width: 0, height: 720 };
game.frameGeometryChanged.emit();
assert.equal(
    clearCalls().length,
    clearsBeforeTrackedInvalidation + 1,
    "an invalid tracked game clears synchronously while the overlay owns focus",
);

const invalid = window({ pid: 0, resourceClass: "invalid", frameGeometry: { x: 0, y: 0, width: 0, height: 1 } });
workspace.windowAdded.emit(invalid);
workspace.activeWindow = invalid;
const reportsBeforeInvalidFocus = reportCalls().length;
const clearsBeforeInvalidFocus = clearCalls().length;
workspace.windowActivated.emit(invalid);
assert.equal(reportCalls().length, reportsBeforeInvalidFocus, "invalid windows are never reported");
assert.equal(clearCalls().length, clearsBeforeInvalidFocus + 1, "invalid non-overlay focus fails closed");

assert.equal(timers.length, 1);
assert.equal(timers[0].interval, 2000);
assert.equal(timers[0].started, true);
workspace.activeWindow = game;
game.frameGeometry = { x: 10, y: 20, width: 1280, height: 720 };
const reportsBeforeKeepalive = reportCalls().length;
timers[0].timeout.emit();
assert.equal(reportCalls().length, reportsBeforeKeepalive + 1, "keepalive renews before the five-second lease");

workspace.activeWindow = overlay;
workspace.windowActivated.emit(overlay);
game.frameGeometry = { x: 10, y: 20, width: 1280, height: 0 };
const clearsBeforeInvalidKeepalive = clearCalls().length;
timers[0].timeout.emit();
assert.equal(
    clearCalls().length,
    clearsBeforeInvalidKeepalive + 1,
    "keepalive clears immediately when the last tracked game becomes invalid",
);

for (const output of [null, {}, { devicePixelRatio: 0 }, { devicePixelRatio: -1 }, { devicePixelRatio: Number.NaN }]) {
    const invalidOutputWindow = window({ pid: 100, output });
    workspace.windowAdded.emit(invalidOutputWindow);
    workspace.activeWindow = invalidOutputWindow;
    const reportsBeforeInvalidOutput = reportCalls().length;
    const clearsBeforeInvalidOutput = clearCalls().length;
    workspace.windowActivated.emit(invalidOutputWindow);
    assert.equal(reportCalls().length, reportsBeforeInvalidOutput, "invalid DPR must never be reported");
    assert.equal(clearCalls().length, clearsBeforeInvalidOutput + 1, "invalid DPR focus fails closed immediately");
}

const outputTrackedGame = window({ pid: 101 });
workspace.windowAdded.emit(outputTrackedGame);
workspace.activeWindow = outputTrackedGame;
workspace.windowActivated.emit(outputTrackedGame);
workspace.activeWindow = overlay;
workspace.windowActivated.emit(overlay);
outputTrackedGame.output = null;
const reportsBeforeMissingTrackedOutput = reportCalls().length;
const clearsBeforeMissingTrackedOutput = clearCalls().length;
outputTrackedGame.outputChanged.emit();
assert.equal(reportCalls().length, reportsBeforeMissingTrackedOutput, "missing tracked output must not be reported");
assert.equal(
    clearCalls().length,
    clearsBeforeMissingTrackedOutput + 1,
    "missing tracked output clears synchronously while overlay owns focus",
);

outputTrackedGame.output = { devicePixelRatio: 1.25 };
workspace.activeWindow = outputTrackedGame;
workspace.windowActivated.emit(outputTrackedGame);
workspace.activeWindow = overlay;
workspace.windowActivated.emit(overlay);
overlay.frameGeometry = { x: 900, y: 900, width: 30, height: 30 };
secondOverlay.frameGeometry = { x: 901, y: 901, width: 31, height: 31 };
outputTrackedGame.output = { devicePixelRatio: 1.5 };
outputTrackedGame.outputChanged.emit();
assert.deepEqual(geometryOf(overlay), [-100, 24, 1920, 1080], "an output change resynchronizes placement");
assert.deepEqual(
    geometryOf(secondOverlay),
    [-100, 24, 1920, 1080],
    "an output change resynchronizes every live overlay",
);
assert.equal(reportCalls().at(-1)[11], "1.5", "an output change refreshes the reported scale");
outputTrackedGame.output = { devicePixelRatio: Number.POSITIVE_INFINITY };
const reportsBeforeInvalidDprKeepalive = reportCalls().length;
const clearsBeforeInvalidDprKeepalive = clearCalls().length;
timers[0].timeout.emit();
assert.equal(reportCalls().length, reportsBeforeInvalidDprKeepalive, "invalid keepalive DPR must not be reported");
assert.equal(
    clearCalls().length,
    clearsBeforeInvalidDprKeepalive + 1,
    "invalid DPR keepalive clears immediately while overlay owns focus",
);

const integralDprWindow = window({ pid: 102, output: { devicePixelRatio: 1.0 } });
workspace.windowAdded.emit(integralDprWindow);
workspace.activeWindow = integralDprWindow;
workspace.windowActivated.emit(integralDprWindow);
assert.equal(reportCalls().at(-1)[11], "1", "integral DPR uses the string wire transport");

const stateGame = window({
    pid: 103,
    resourceClass: "state-game",
    frameGeometry: { x: 100, y: 200, width: 900, height: 600 },
});
workspace.windowAdded.emit(stateGame);
workspace.activeWindow = stateGame;
workspace.windowActivated.emit(stateGame);
workspace.activeWindow = overlay;
workspace.windowActivated.emit(overlay);

const reportsBeforeMinimize = reportCalls().length;
const clearsBeforeMinimize = clearCalls().length;
stateGame.minimized = true;
stateGame.minimizedChanged.emit();
assert.equal(reportCalls().length, reportsBeforeMinimize, "a minimized game must not be reported");
assert.equal(clearCalls().length, clearsBeforeMinimize + 1, "minimizing the tracked game clears immediately");
const geometryBeforeMinimizedChange = geometryOf(overlay);
stateGame.frameGeometry = { x: 101, y: 201, width: 901, height: 601 };
stateGame.frameGeometryChanged.emit();
assert.deepEqual(
    geometryOf(overlay),
    geometryBeforeMinimizedChange,
    "a minimized game must no longer drive overlay placement",
);

stateGame.minimized = false;
workspace.activeWindow = stateGame;
stateGame.minimizedChanged.emit();
assert.equal(reportCalls().at(-1)[6], "state-game", "an active restored game becomes reportable again");
assert.deepEqual(geometryOf(overlay), [101, 201, 901, 601]);

const reportsBeforeHidden = reportCalls().length;
const clearsBeforeHidden = clearCalls().length;
stateGame.hidden = true;
stateGame.windowHidden.emit(stateGame);
assert.equal(reportCalls().length, reportsBeforeHidden, "a hidden game must not be reported");
assert.equal(clearCalls().length, clearsBeforeHidden + 1, "hiding the tracked game clears immediately");
const geometryBeforeHiddenChange = geometryOf(overlay);
stateGame.frameGeometry = { x: 102, y: 202, width: 902, height: 602 };
stateGame.frameGeometryChanged.emit();
assert.deepEqual(
    geometryOf(overlay),
    geometryBeforeHiddenChange,
    "a hidden game must no longer drive overlay placement",
);

stateGame.hidden = false;
stateGame.windowShown.emit(stateGame);
assert.equal(reportCalls().at(-1)[6], "state-game", "showing the active game reports it again");
assert.deepEqual(geometryOf(overlay), [102, 202, 902, 602]);

workspace.activeWindow = overlay;
workspace.windowActivated.emit(overlay);
const reportsBeforeClose = reportCalls().length;
const clearsBeforeClose = clearCalls().length;
stateGame.closed.emit();
assert.equal(reportCalls().length, reportsBeforeClose, "a closed game must not be reported");
assert.equal(clearCalls().length, clearsBeforeClose + 1, "closing the tracked game clears immediately");
const geometryBeforeClosedChange = geometryOf(overlay);
stateGame.frameGeometry = { x: 103, y: 203, width: 903, height: 603 };
stateGame.frameGeometryChanged.emit();
assert.deepEqual(
    geometryOf(overlay),
    geometryBeforeClosedChange,
    "a closed game must no longer drive overlay placement",
);

const unmanagedGame = window({ pid: 104, resourceClass: "unmanaged-game", managed: false });
workspace.windowAdded.emit(unmanagedGame);
workspace.activeWindow = unmanagedGame;
const reportsBeforeUnmanaged = reportCalls().length;
const clearsBeforeUnmanaged = clearCalls().length;
workspace.windowActivated.emit(unmanagedGame);
assert.equal(reportCalls().length, reportsBeforeUnmanaged, "an unmanaged window must not be reported as a game");
assert.equal(clearCalls().length, clearsBeforeUnmanaged + 1, "unmanaged focus clears immediately");

const cleanupGame = window({
    pid: 105,
    resourceClass: "cleanup-game",
    frameGeometry: { x: 300, y: 400, width: 1000, height: 700 },
});
workspace.windowAdded.emit(cleanupGame);
workspace.activeWindow = cleanupGame;
workspace.windowActivated.emit(cleanupGame);
const closedOverlay = window({ pid: 106, resourceClass: "io.github.overcrow.Overlay" });
workspace.windowAdded.emit(closedOverlay);
closedOverlay.closed.emit();
const closedOverlayGeometry = geometryOf(closedOverlay);
cleanupGame.frameGeometry = { x: 301, y: 401, width: 1001, height: 701 };
cleanupGame.frameGeometryChanged.emit();
assert.deepEqual(
    geometryOf(closedOverlay),
    closedOverlayGeometry,
    "a closed overlay must be removed from placement synchronization",
);

const deletedOverlay = window({ pid: 106, resourceClass: "io.github.overcrow.Overlay" });
workspace.windowAdded.emit(deletedOverlay);
deletedOverlay.deleted = true;
workspace.windowRemoved.emit(deletedOverlay);
const deletedOverlayGeometry = geometryOf(deletedOverlay);
deletedOverlay.deleted = false;
cleanupGame.frameGeometry = { x: 302, y: 402, width: 1002, height: 702 };
cleanupGame.frameGeometryChanged.emit();
assert.deepEqual(
    geometryOf(deletedOverlay),
    deletedOverlayGeometry,
    "a deleted overlay must be removed from placement synchronization",
);

const deletedGame = window({ pid: 107, resourceClass: "deleted-game" });
workspace.windowAdded.emit(deletedGame);
workspace.activeWindow = deletedGame;
workspace.windowActivated.emit(deletedGame);
workspace.activeWindow = overlay;
workspace.windowActivated.emit(overlay);
const reportsBeforeDeleted = reportCalls().length;
const clearsBeforeDeleted = clearCalls().length;
deletedGame.deleted = true;
workspace.windowRemoved.emit(deletedGame);
assert.equal(reportCalls().length, reportsBeforeDeleted, "a deleted game must not be reported");
assert.equal(clearCalls().length, clearsBeforeDeleted + 1, "deleting the tracked game clears immediately");

const recursionGame = window({
    pid: 108,
    resourceClass: "recursion-game",
    frameGeometry: { x: 410, y: 510, width: 1110, height: 710 },
});
workspace.windowAdded.emit(recursionGame);
workspace.activeWindow = recursionGame;
workspace.windowActivated.emit(recursionGame);
const recursionOverlay = window({
    pid: 109,
    resourceClass: "io.github.overcrow.Overlay",
    frameGeometry: { x: 0, y: 0, width: 1, height: 1 },
});
let recursionOverlayGeometry = recursionOverlay.frameGeometry;
let recursionOverlayAssignments = 0;
Object.defineProperty(recursionOverlay, "frameGeometry", {
    configurable: true,
    get: () => recursionOverlayGeometry,
    set: (geometry) => {
        recursionOverlayAssignments += 1;
        if (recursionOverlayAssignments > 10) {
            throw new Error("recursive overlay geometry assignment");
        }
        recursionOverlay.frameGeometryChanged.emit();
        recursionOverlayGeometry = geometry;
    },
});
workspace.windowAdded.emit(recursionOverlay);
assert.equal(recursionOverlayAssignments, 1, "the new overlay receives one initial geometry assignment");
recursionGame.frameGeometry = { x: 411, y: 511, width: 1111, height: 711 };
recursionGame.frameGeometryChanged.emit();
assert.equal(recursionOverlayAssignments, 2, "a game update must assign overlay geometry without recursive writes");
assert.deepEqual(geometryOf(recursionOverlay), [411, 511, 1111, 711]);

assert.match(
    source,
    /return rounded \| 0;/,
    "validated integer arguments must use a QV4 int32-producing bitwise coercion",
);
assert.doesNotMatch(source, /return rounded;/, "Math.round alone leaves a QV4 double");
assert.doesNotMatch(source, /window\.output\s*\?[^;]+:\s*1\.0/, "missing output must not fall back to a literal scale");

console.log("KWin bridge smoke test passed");
