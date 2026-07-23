"use strict";

const DBUS_SERVICE = "io.github.overcrow.Core1";
const DBUS_PATH = "/io/github/overcrow/Core1";
const DBUS_INTERFACE = "io.github.overcrow.Core1";
const OVERLAY_APP_ID = "io.github.overcrow.Overlay";
const KEEPALIVE_MS = 2000;
const INT32_MAX = 2147483647;
const INT32_MIN = -2147483648;

let lastReportableWindow = null;
let lastReportSignature = null;
let overlayWindows = [];
let overlayBeingPlaced = null;

function appId(window) {
    return window ? String(window.resourceClass || "") : "";
}

function isOverlay(window) {
    return appId(window) === OVERLAY_APP_ID;
}

function forgetOverlay(window) {
    overlayWindows = overlayWindows.filter((overlay) => overlay !== window);
}

function rememberOverlay(window) {
    if (overlayWindows.indexOf(window) === -1) {
        overlayWindows.push(window);
    }
}

function forgetTrackedWindow(window) {
    forgetOverlay(window);
    if (window === lastReportableWindow) {
        lastReportableWindow = null;
        clearWindow();
    }
}

function sameGeometry(left, right) {
    return (
        left &&
        left.x === right.x &&
        left.y === right.y &&
        left.width === right.width &&
        left.height === right.height
    );
}

function reportableGeometry(window) {
    if (!normalizedWindow(window)) {
        return null;
    }

    const geometry = window.frameGeometry;
    return {
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
    };
}

function configureOverlay(window) {
    if (!window || !isOverlay(window)) {
        return false;
    }
    if (window.deleted) {
        return false;
    }

    rememberOverlay(window);

    window.keepAbove = true;
    window.skipTaskbar = true;
    window.skipPager = true;
    window.skipSwitcher = true;
    window.noBorder = true;

    const geometry = reportableGeometry(lastReportableWindow);
    if (geometry && window !== overlayBeingPlaced && !sameGeometry(window.frameGeometry, geometry)) {
        const previouslyPlacedOverlay = overlayBeingPlaced;
        overlayBeingPlaced = window;
        try {
            window.frameGeometry = geometry;
        } finally {
            overlayBeingPlaced = previouslyPlacedOverlay;
        }
    }
    return true;
}

function syncOverlays() {
    for (const overlay of overlayWindows.slice()) {
        configureOverlay(overlay);
    }
}

function finiteInteger(value, minimum, maximum) {
    const number = Number(value);
    if (!Number.isFinite(number)) {
        return null;
    }
    const rounded = Math.round(number);
    if (rounded < minimum || rounded > maximum) {
        return null;
    }
    return rounded | 0;
}

function normalizedWindow(window) {
    if (
        !window ||
        window.deleted ||
        window.managed === false ||
        window.minimized === true ||
        window.hidden === true ||
        isOverlay(window)
    ) {
        return null;
    }

    const geometry = window.frameGeometry;
    if (!window.output) {
        return null;
    }
    const pid = finiteInteger(window.pid, 1, INT32_MAX);
    const x = geometry ? finiteInteger(geometry.x, INT32_MIN, INT32_MAX) : null;
    const y = geometry ? finiteInteger(geometry.y, INT32_MIN, INT32_MAX) : null;
    const width = geometry ? finiteInteger(geometry.width, 1, INT32_MAX) : null;
    const height = geometry ? finiteInteger(geometry.height, 1, INT32_MAX) : null;
    const id = appId(window);
    const scale = window.output.devicePixelRatio;

    if (pid === null || x === null || y === null || width === null || height === null) {
        return null;
    }
    if (!id || typeof scale !== "number" || !Number.isFinite(scale) || scale <= 0) {
        return null;
    }

    return {
        pid,
        title: String(window.caption || ""),
        appId: id,
        x,
        y,
        width,
        height,
        scale: String(scale),
    };
}

function clearWindow() {
    lastReportSignature = null;
    callDBus(DBUS_SERVICE, DBUS_PATH, DBUS_INTERFACE, "ClearWindow");
}

function reportWindow(window, force) {
    const report = normalizedWindow(window);
    if (!report) {
        return false;
    }

    lastReportableWindow = window;
    syncOverlays();

    const signature = JSON.stringify(report);
    if (!force && signature === lastReportSignature) {
        return true;
    }

    lastReportSignature = signature;
    callDBus(
        DBUS_SERVICE,
        DBUS_PATH,
        DBUS_INTERFACE,
        "ReportWindow",
        report.pid,
        report.title,
        report.appId,
        report.x,
        report.y,
        report.width,
        report.height,
        report.scale,
    );
    return true;
}

function handleActiveWindow(window) {
    if (isOverlay(window)) {
        configureOverlay(window);
        if (!reportWindow(lastReportableWindow, true)) {
            lastReportableWindow = null;
            clearWindow();
        }
        return;
    }

    if (!reportWindow(window, true)) {
        lastReportableWindow = null;
        clearWindow();
    }
}

function handleTrackedChange(window) {
    if (configureOverlay(window)) {
        return;
    }
    if (window === workspace.activeWindow) {
        handleActiveWindow(window);
    } else if (window === lastReportableWindow && isOverlay(workspace.activeWindow)) {
        if (!reportWindow(window, false)) {
            lastReportableWindow = null;
            clearWindow();
        }
    }
}

function connectSignal(signal, callback) {
    if (signal && typeof signal.connect === "function") {
        signal.connect(callback);
    }
}

function watchWindow(window) {
    if (!window) {
        return;
    }
    configureOverlay(window);
    connectSignal(window.frameGeometryChanged, () => handleTrackedChange(window));
    connectSignal(window.captionChanged, () => handleTrackedChange(window));
    connectSignal(window.windowClassChanged, () => handleTrackedChange(window));
    connectSignal(window.outputChanged, () => handleTrackedChange(window));
    connectSignal(window.minimizedChanged, () => handleTrackedChange(window));
    connectSignal(window.windowHidden, () => handleTrackedChange(window));
    connectSignal(window.windowShown, () => handleTrackedChange(window));
    connectSignal(window.closed, () => forgetTrackedWindow(window));
}

for (const window of workspace.stackingOrder || []) {
    watchWindow(window);
}

connectSignal(workspace.windowAdded, (window) => {
    watchWindow(window);
    if (window === workspace.activeWindow) {
        handleActiveWindow(window);
    }
});
connectSignal(workspace.windowRemoved, (window) => forgetTrackedWindow(window));
connectSignal(workspace.windowActivated, (window) => {
    handleActiveWindow(window || workspace.activeWindow);
});

registerShortcut("OverCrowToggleOverlay", "Toggle OverCrow overlay", "Meta+Alt+O", () => {
    callDBus(DBUS_SERVICE, DBUS_PATH, DBUS_INTERFACE, "ToggleOverlay");
});

handleActiveWindow(workspace.activeWindow);

const keepaliveTimer = new QTimer();
keepaliveTimer.interval = KEEPALIVE_MS;
keepaliveTimer.timeout.connect(() => handleActiveWindow(workspace.activeWindow));
keepaliveTimer.start();
