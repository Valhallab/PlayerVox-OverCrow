#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

fail() {
    printf '%s\n' "disabled entry-point smoke test failed: $*" >&2
    exit 1
}

cargo build --locked --offline \
    -p overcrow-core \
    -p overcrow-overlay \
    -p overcrow-hyprland

temp_root=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-disabled-smoke.XXXXXX")
trap 'rm -rf "$temp_root"' EXIT HUP INT TERM

mkdir -p "$temp_root/home" "$temp_root/fake-bin" "$temp_root/tmp"
systemctl_marker=$temp_root/systemctl-called
# shellcheck disable=SC2016 # Emit a literal variable reference into the stub.
printf '%s\n' \
    '#!/bin/sh' \
    ': > "$OVERCROW_SYSTEMCTL_MARKER"' \
    'exit 99' >"$temp_root/fake-bin/systemctl"
chmod 700 "$temp_root/fake-bin/systemctl"

isolated_home=$temp_root/home
isolated_config=$temp_root/missing-config
isolated_runtime=$temp_root/missing-runtime
isolated_tmp=$temp_root/tmp
isolated_path=$temp_root/fake-bin:/usr/bin:/bin
poisoned_session_bus=unix:path=$temp_root/missing-session-bus

# Seed selectors in the parent so the probe below proves the wrapper removes
# them instead of passing only because the invoking shell happened to lack them.
export WAYLAND_SOCKET=parent-wayland-socket
export DBUS_STARTER_ADDRESS=unix:path=/parent/session-bus
export DBUS_STARTER_BUS_TYPE=session
export SYSTEMD_BUS_ADDRESS=unix:path=/parent/systemd-bus
export LISTEN_FDS=8
export LISTEN_PID=$$
export LISTEN_FDNAMES=wayland:dbus:systemd
export NOTIFY_SOCKET=/parent/notify-socket
export WATCHDOG_PID=$$
export WATCHDOG_USEC=1000000

# The poisoned session bus and runtime isolate fixed-path systemctl regressions;
# the PATH stub records the current PATH-based command.

[ ! -e "$isolated_config" ] || fail "isolated config root must be missing"

run_child() {
    exec /usr/bin/env -i \
        HOME="$isolated_home" \
        XDG_CONFIG_HOME="$isolated_config" \
        XDG_RUNTIME_DIR="$isolated_runtime" \
        TMPDIR="$isolated_tmp" \
        XDG_SESSION_TYPE=wayland \
        XDG_CURRENT_DESKTOP=Hyprland \
        HYPRLAND_INSTANCE_SIGNATURE=overcrow-disabled-smoke \
        WAYLAND_DISPLAY=overcrow-disabled-smoke-wayland \
        DISPLAY=overcrow-disabled-smoke-x11 \
        DBUS_SESSION_BUS_ADDRESS="$poisoned_session_bus" \
        OVERCROW_SYSTEMCTL_MARKER="$systemctl_marker" \
        PATH="$isolated_path" \
        LANG=C \
        LC_ALL=C \
        "$@"
}

child_environment=$temp_root/child-environment
(run_child /usr/bin/env) >"$child_environment"
for dangerous_name in \
    WAYLAND_SOCKET \
    DBUS_STARTER_ADDRESS \
    DBUS_STARTER_BUS_TYPE \
    SYSTEMD_BUS_ADDRESS \
    LISTEN_FDS \
    LISTEN_PID \
    LISTEN_FDNAMES \
    NOTIFY_SOCKET \
    WATCHDOG_PID \
    WATCHDOG_USEC
do
    if grep -q "^$dangerous_name=" "$child_environment"; then
        fail "child inherited dangerous variable $dangerous_name"
    fi
done

expected_environment=$temp_root/expected-child-environment
printf '%s\n' \
    "DBUS_SESSION_BUS_ADDRESS=$poisoned_session_bus" \
    'DISPLAY=overcrow-disabled-smoke-x11' \
    "HOME=$isolated_home" \
    'HYPRLAND_INSTANCE_SIGNATURE=overcrow-disabled-smoke' \
    'LANG=C' \
    'LC_ALL=C' \
    "OVERCROW_SYSTEMCTL_MARKER=$systemctl_marker" \
    "PATH=$isolated_path" \
    "TMPDIR=$isolated_tmp" \
    'WAYLAND_DISPLAY=overcrow-disabled-smoke-wayland' \
    "XDG_CONFIG_HOME=$isolated_config" \
    'XDG_CURRENT_DESKTOP=Hyprland' \
    "XDG_RUNTIME_DIR=$isolated_runtime" \
    'XDG_SESSION_TYPE=wayland' |
    /usr/bin/sort >"$expected_environment"
/usr/bin/sort "$child_environment" >"$child_environment.sorted"
if ! /usr/bin/cmp -s "$expected_environment" "$child_environment.sorted"; then
    /usr/bin/diff -u "$expected_environment" "$child_environment.sorted" >&2 || true
    fail 'child environment differs from the strict allowlist'
fi

run_bounded() {
    name=$1
    binary=$2
    timeout_marker=$temp_root/$name-timeout
    stdout=$temp_root/$name.stdout
    stderr=$temp_root/$name.stderr

    (run_child "$binary") >"$stdout" 2>"$stderr" &
    pid=$!
    (
        sleep 3
        if kill -0 "$pid" 2>/dev/null; then
            : >"$timeout_marker"
            kill -TERM "$pid" 2>/dev/null || true
            sleep 1
            kill -KILL "$pid" 2>/dev/null || true
        fi
    ) &
    watchdog=$!

    if wait "$pid"; then
        status=0
    else
        status=$?
    fi

    kill -TERM "$watchdog" 2>/dev/null || true
    wait "$watchdog" 2>/dev/null || true

    if [ -e "$timeout_marker" ]; then
        fail "$name did not exit within three seconds"
    fi
    if [ "$status" -ne 0 ]; then
        sed -n '1,80p' "$stderr" >&2
        fail "$name exited with status $status"
    fi
}

run_bounded overcrow-core target/debug/overcrow-core
run_bounded overcrow-overlay target/debug/overcrow-overlay
run_bounded overcrow-hyprland target/debug/overcrow-hyprland

[ ! -e "$systemctl_marker" ] || fail 'an entry point attempted to invoke systemctl'

printf '%s\n' 'disabled runtime entry points: inert'
