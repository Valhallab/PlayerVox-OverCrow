#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
tmpdir=$(mktemp -d)
trap '/usr/bin/rm -rf "$tmpdir"' EXIT HUP INT TERM

cargo build --locked --offline -p overcrow-control-ui --bin overcrow-control >/dev/null
diagnostic_helper_source="$repo_root/target/debug/overcrow-control"

install_diagnostic_helper() {
    helper_home=$1
    mkdir -p "$helper_home/.local/bin"
    /usr/bin/install -m 0755 "$diagnostic_helper_source" \
        "$helper_home/.local/bin/overcrow-control"
}

snapshot_tree() {
    for snapshot_root in "$@"; do
        /usr/bin/find "$snapshot_root" -printf '%p|%y|%m|%s|%T@|%C@\n'
        /usr/bin/find "$snapshot_root" -type f -exec /usr/bin/cksum {} \;
    done | /usr/bin/sort
}

assert_tree_unchanged() {
    tree_before=$1
    shift
    tree_after=$(snapshot_tree "$@")
    if [ "$tree_before" != "$tree_after" ]; then
        printf '%s\n' "diagnostic changed a HOME/XDG tree" >&2
        exit 1
    fi
}

assert_read_only_log() {
    checked_log=$1
    while IFS= read -r invocation; do
        case "$invocation" in
            "timeout --signal=TERM --kill-after=1s 3s id -u"|\
            "id -u"|\
            "timeout --signal=TERM --kill-after=1s 3s busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus NameHasOwner s io.github.overcrow.Core1"|\
            "busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus NameHasOwner s io.github.overcrow.Core1"|\
            "timeout --signal=TERM --kill-after=1s 3s busctl --user call io.github.overcrow.Core1 /io/github/overcrow/Core1 io.github.overcrow.Core1 ShortcutAvailability"|\
            "busctl --user call io.github.overcrow.Core1 /io/github/overcrow/Core1 io.github.overcrow.Core1 ShortcutAvailability"|\
            "timeout --signal=TERM --kill-after=1s 3s gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner io.github.overcrow.Core1"|\
            "gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner io.github.overcrow.Core1"|\
            "timeout --signal=TERM --kill-after=1s 3s systemctl --user is-active overcrow-core.service"|\
            "systemctl --user is-active overcrow-core.service"|\
            "timeout --signal=TERM --kill-after=1s 3s systemctl --user is-active overcrow-hyprland.service"|\
            "systemctl --user is-active overcrow-hyprland.service"|\
            "timeout --signal=TERM --kill-after=1s 3s pgrep -u 1000 -f ^([^[:space:]]*/)?overcrow-core([[:space:]]|$)"|\
            "pgrep -u 1000 -f ^([^[:space:]]*/)?overcrow-core([[:space:]]|$)"|\
            "timeout --signal=TERM --kill-after=1s 3s pgrep -u 1000 -f ^([^[:space:]]*/)?overcrow-overlay([[:space:]]|$)"|\
            "pgrep -u 1000 -f ^([^[:space:]]*/)?overcrow-overlay([[:space:]]|$)"|\
            "timeout --signal=TERM --kill-after=1s 3s kpackagetool6 --type KWin/Script --show io.github.overcrow.kwin"|\
            "kpackagetool6 --type KWin/Script --show io.github.overcrow.kwin"|\
            "timeout --signal=TERM --kill-after=1s 3s kreadconfig6 --file kwinrc --group Plugins --key io.github.overcrow.kwinEnabled"|\
            "kreadconfig6 --file kwinrc --group Plugins --key io.github.overcrow.kwinEnabled"|\
            "timeout --signal=TERM --kill-after=1s 3s hyprctl version"|\
            "hyprctl version"|\
            "timeout --signal=TERM --kill-after=1s 3s hyprctl -j clients"|\
            "hyprctl -j clients"|\
            "timeout --signal=TERM --kill-after=1s 3s hyprctl -j binds"|\
            "hyprctl -j binds"|\
            "timeout --signal=TERM --kill-after=1s 3s hyprctl configerrors"|\
            "hyprctl configerrors"|\
            "timeout --signal=TERM --kill-after=1s 3s jq "*|\
            "timeout --signal=TERM --kill-after=1s 3s stat "*|\
            "timeout --signal=TERM --kill-after=1s 3s /usr/bin/stat "*|\
            "timeout --signal=TERM --kill-after=1s 3s /usr/bin/head "*|\
            "timeout --signal=TERM --kill-after=1s 3s /bin/sh -c "*|\
            "timeout --signal=TERM --kill-after=1s 3s /usr/bin/gawk "*|\
            "timeout --signal=TERM --kill-after=1s 3s sha256sum "*|\
            "timeout --signal=TERM --kill-after=1s 3s readlink "*|\
            "timeout --signal=TERM --kill-after=1s 3s /usr/bin/cmp "*|\
            "timeout --signal=TERM --kill-after=1s 3s "*/overcrow-control\ --overcrow-diagnose-settings-v1|\
            "timeout --signal=TERM --kill-after=1s 3s wayland-info"|\
            "timeout --signal=TERM --kill-after=1s 3s xprop -root _NET_SUPPORTING_WM_CHECK"|\
            "timeout --signal=TERM --kill-after=1s 3s xprop -root _NET_ACTIVE_WINDOW"|\
            "wayland-info "|\
            "xprop -root _NET_SUPPORTING_WM_CHECK"|\
            "xprop -root _NET_ACTIVE_WINDOW")
                ;;
            *)
                printf '%s\n' "non-read-only or unexpected invocation: $invocation" >&2
                exit 1
                ;;
        esac
    done < "$checked_log"
}

assert_bounded_probe_pairs() {
    checked_log=$1
    previous_invocation=
    while IFS= read -r invocation; do
        case "$invocation" in
            "id "*|"busctl "*|"gdbus "*|"systemctl "*|"pgrep "*|\
            "kpackagetool6 "*|"kreadconfig6 "*|"hyprctl "*|"wayland-info "*|"xprop "*|\
            "/usr/bin/cmp "*|\
            /*/overcrow-control\ --overcrow-diagnose-settings-v1)
                probe_invocation=$invocation
                if [ "$invocation" = "wayland-info " ]; then
                    probe_invocation=wayland-info
                fi
                expected_wrapper="timeout --signal=TERM --kill-after=1s 3s $probe_invocation"
                if [ "$previous_invocation" != "$expected_wrapper" ]; then
                    printf '%s\n' "orphan probe invocation: $invocation" >&2
                    exit 1
                fi
                ;;
        esac
        previous_invocation=$invocation
    done < "$checked_log"
}

orphan_log="$tmpdir/orphan.log"
printf '%s\n' \
    "busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus NameHasOwner s io.github.overcrow.Core1" \
    > "$orphan_log"
if (assert_bounded_probe_pairs "$orphan_log") 2>/dev/null; then
    printf '%s\n' "pair checker accepted an orphan probe" >&2
    exit 1
fi

home="$tmpdir/home"
data_home="$tmpdir/data"
config_home="$tmpdir/config"
stub_bin="$tmpdir/stubs"
present_log="$tmpdir/present.log"
mkdir -p "$home" "$data_home" "$config_home" "$stub_bin"
install_diagnostic_helper "$home"
printf '%s\n' home-canary > "$home/canary"
printf '%s\n' data-canary > "$data_home/canary"
printf '%s\n' config-canary > "$config_home/canary"
mkdir -p "$config_home/overcrow"
printf '%s\n' \
    '{"schema_version":1,"enabled":true,"selected_steam_app_ids":[1623730,620],"manual_games":[],"shortcut":{"enabled":true,"accelerator":"Meta+Alt+O"}}' \
    > "$config_home/overcrow/settings.json"
chmod 600 "$config_home/overcrow/settings.json"
: > "$present_log"

for command_name in \
    id busctl systemctl pgrep kpackagetool6 kreadconfig6 timeout wayland-info xprop \
    install cp mv rm mkdir touch chmod chown kwriteconfig6 qdbus6 qdbus
do
    ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" "$stub_bin/$command_name"
done
ln -s /usr/bin/jq "$stub_bin/jq"
ln -s /usr/bin/stat "$stub_bin/stat"
ln -s /usr/bin/sha256sum "$stub_bin/sha256sum"
ln -s /usr/bin/readlink "$stub_bin/readlink"

present_tree_before=$(snapshot_tree "$home" "$data_home" "$config_home")
present_output=$(
    HOME="$home" \
    XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" \
    XDG_SESSION_TYPE=wayland \
    XDG_CURRENT_DESKTOP=KDE \
    DESKTOP_SESSION=plasmawayland \
    DISPLAY=:99 \
    OVERCROW_DIAG_STUB_LOG="$present_log" \
    PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)

printf '%s\n' "$present_output" | grep -Fq "XDG_SESSION_TYPE: wayland"
printf '%s\n' "$present_output" | grep -Fq "XDG_CURRENT_DESKTOP: KDE"
printf '%s\n' "$present_output" | grep -Fq "DESKTOP_SESSION: plasmawayland"
printf '%s\n' "$present_output" | grep -Fq "D-Bus io.github.overcrow.Core1 owner: yes"
printf '%s\n' "$present_output" | grep -Fq "overcrow-core.service: active"
printf '%s\n' "$present_output" | grep -Fq "overcrow-core process: running (PIDs: 4242 4243)"
printf '%s\n' "$present_output" | grep -Fq "overcrow-overlay process: running (PIDs: 5252)"
printf '%s\n' "$present_output" | grep -Fq "Lifecycle settings: valid"
printf '%s\n' "$present_output" | grep -Fq "OverCrow master state: enabled"
printf '%s\n' "$present_output" | grep -Fq "Selected games: 2"
printf '%s\n' "$present_output" | grep -Fq "Shortcut availability: available"
printf '%s\n' "$present_output" | grep -Fq "Legacy OverCrow artifacts: none"
printf '%s\n' "$present_output" | grep -Fq "KWin package io.github.overcrow.kwin: installed"
printf '%s\n' "$present_output" | grep -Fq "KWin package enabled: yes"
printf '%s\n' "$present_output" | grep -Fq "interface: 'wl_compositor'"
printf '%s\n' "$present_output" | grep -Fq "interface: 'xdg_wm_base'"
printf '%s\n' "$present_output" | grep -Fq "interface: 'org_kde_plasma_shell'"
if printf '%s\n' "$present_output" | grep -Fq "irrelevant_test_global"; then
    printf '%s\n' "diagnostic leaked an irrelevant Wayland global" >&2
    exit 1
fi
printf '%s\n' "$present_output" | grep -Fq "X11 EWMH _NET_SUPPORTING_WM_CHECK: present"
printf '%s\n' "$present_output" | grep -Fq "X11 EWMH _NET_ACTIVE_WINDOW: absent"
if printf '%s\n' "$present_output" | grep -Fq "window id #"; then
    printf '%s\n' "diagnostic leaked raw xprop output" >&2
    exit 1
fi
assert_tree_unchanged "$present_tree_before" "$home" "$data_home" "$config_home"
assert_read_only_log "$present_log"
assert_bounded_probe_pairs "$present_log"
if grep -E '(stat|jq).*settings\.json' "$present_log"; then
    printf '%s\n' 'diagnostic inspected settings through a path-based stat/jq race' >&2
    exit 1
fi

invalid_home="$tmpdir/invalid-settings-home"
invalid_data="$tmpdir/invalid-settings-data"
invalid_config="$tmpdir/invalid-settings-config"
invalid_log="$tmpdir/invalid-settings.log"
mkdir -p "$invalid_home" "$invalid_data" "$invalid_config/overcrow"
install_diagnostic_helper "$invalid_home"
printf '%s\n' '{"schema_version":1,"enabled":true,"selected_steam_app_ids":[0],"manual_games":[],"shortcut":{"enabled":true,"accelerator":"Alt+O"}}' \
    > "$invalid_config/overcrow/settings.json"
chmod 600 "$invalid_config/overcrow/settings.json"
: > "$invalid_log"
invalid_tree_before=$(snapshot_tree "$invalid_home" "$invalid_data" "$invalid_config")
invalid_output=$(
    HOME="$invalid_home" XDG_DATA_HOME="$invalid_data" \
    XDG_CONFIG_HOME="$invalid_config" XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_STUB_LOG="$invalid_log" PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$invalid_output" | \
    grep -Fq 'Lifecycle settings: invalid or unsafe (runtime defaults to disabled)'
printf '%s\n' "$invalid_output" | grep -Fq 'OverCrow master state: disabled'
printf '%s\n' "$invalid_output" | grep -Fq 'Selected games: 0'
assert_tree_unchanged "$invalid_tree_before" \
    "$invalid_home" "$invalid_data" "$invalid_config"
assert_read_only_log "$invalid_log"
assert_bounded_probe_pairs "$invalid_log"

relative_home="$tmpdir/relative-home"
relative_data="$tmpdir/relative-data"
relative_log="$tmpdir/relative.log"
mkdir -p "$relative_home/.config/overcrow" "$relative_data"
install_diagnostic_helper "$relative_home"
cp "$config_home/overcrow/settings.json" \
    "$relative_home/.config/overcrow/settings.json"
chmod 600 "$relative_home/.config/overcrow/settings.json"
: > "$relative_log"
relative_output=$(
    HOME="$relative_home" XDG_DATA_HOME="$relative_data" \
    XDG_CONFIG_HOME=relative-xdg XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_STUB_LOG="$relative_log" PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
if ! printf '%s\n' "$relative_output" | grep -Fq 'Lifecycle settings: valid'; then
    printf '%s\n' 'diagnostic did not fall back from relative XDG_CONFIG_HOME to absolute HOME' >&2
    exit 1
fi
printf '%s\n' "$relative_output" | grep -Fq 'OverCrow master state: enabled'
printf '%s\n' "$relative_output" | grep -Fq 'Selected games: 2'

# A native package installs the validator in /usr/bin, while this source
# diagnostic remains useful from a checkout. Rewrite only that fixed prefix so
# the packaged layout can be exercised without touching the host filesystem.
packaged_home="$tmpdir/packaged-home"
packaged_bin="$tmpdir/packaged-bin"
packaged_diagnose="$tmpdir/packaged-diagnose.sh"
packaged_log="$tmpdir/packaged.log"
mkdir -p "$packaged_home" "$packaged_bin"
/usr/bin/install -m 0755 "$diagnostic_helper_source" \
    "$packaged_bin/overcrow-control"
/usr/bin/sed "s|/usr/bin/overcrow-control|$packaged_bin/overcrow-control|g" \
    "$repo_root/scripts/diagnose.sh" > "$packaged_diagnose"
chmod 0755 "$packaged_diagnose"
: > "$packaged_log"
packaged_output=$(
    HOME="$packaged_home" XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_STUB_LOG="$packaged_log" PATH="$stub_bin" \
        "$packaged_diagnose"
)
printf '%s\n' "$packaged_output" | grep -Fq 'Lifecycle settings: valid'
printf '%s\n' "$packaged_output" | grep -Fq 'OverCrow master state: enabled'
printf '%s\n' "$packaged_output" | grep -Fq 'Selected games: 2'
assert_read_only_log "$packaged_log"
assert_bounded_probe_pairs "$packaged_log"

relative_only_output=$(
    HOME=relative-home XDG_DATA_HOME=relative-data XDG_CONFIG_HOME=relative-xdg \
    XDG_SESSION_TYPE=x11 OVERCROW_DIAG_STUB_LOG="$tmpdir/relative-only.log" \
    PATH="$stub_bin" "$repo_root/scripts/diagnose.sh"
)
if ! printf '%s\n' "$relative_only_output" | \
    grep -Fq 'Lifecycle settings: unavailable (config home unknown)'; then
    printf '%s\n' 'diagnostic accepted relative HOME/XDG authority roots' >&2
    exit 1
fi
printf '%s\n' "$relative_only_output" | grep -Fq 'OverCrow master state: unavailable'
printf '%s\n' "$relative_only_output" | grep -Fq 'Selected games: unavailable'

legacy_home="$tmpdir/legacy-home"
legacy_data="$tmpdir/legacy-data"
legacy_config="$tmpdir/legacy-config"
legacy_log="$tmpdir/legacy.log"
mkdir -p \
    "$legacy_home" "$legacy_data/systemd/user" "$legacy_data/applications" \
    "$legacy_config/systemd/user/default.target.wants" "$legacy_config/hypr"
: > "$legacy_data/systemd/user/overcrow-core.service"
ln -s "$legacy_data/systemd/user/overcrow-core.service" \
    "$legacy_config/systemd/user/default.target.wants/overcrow-core.service"
cp "$repo_root/tests/fixtures/legacy-overlay.desktop" \
    "$legacy_data/applications/io.github.overcrow.Overlay.desktop"
printf '%s\n' \
    '# Managed by OverCrow. Changes are replaced by the installer.' \
    'exec-once = systemctl --user import-environment HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP WAYLAND_DISPLAY XDG_RUNTIME_DIR && systemctl --user restart overcrow-hyprland.service' \
    'windowrule = match:class ^(io\.github\.overcrow\.Overlay)$, float on, no_focus on, no_initial_focus on, no_follow_mouse on, focus_on_activate off, decorate off, no_anim on, no_blur on, no_dim on, no_shadow on, no_shortcuts_inhibit on, suppress_event maximize fullscreen' \
    'windowrule = match:tag overcrow-interactive, no_focus off, no_follow_mouse off' \
    'windowrule = match:tag overcrow-game-input-blocked, no_focus on' \
    "bind = SUPER ALT, O, exec, $legacy_home/.local/bin/overcrowctl toggle" \
    > "$legacy_config/hypr/overcrow.conf"
printf '%s\n' \
    '-- BEGIN OVERCROW MANAGED' \
    'require("overcrow")' \
    '-- END OVERCROW MANAGED' \
    > "$legacy_config/hypr/hyprland.lua"
sed "s|@OVERCROWCTL@|$legacy_home/.local/bin/overcrowctl|g" \
    "$repo_root/tests/fixtures/legacy-overcrow.lua.in" \
    > "$legacy_config/hypr/overcrow.lua"
: > "$legacy_log"
legacy_tree_before=$(snapshot_tree "$legacy_home" "$legacy_data" "$legacy_config")
legacy_output=$(
    HOME="$legacy_home" XDG_DATA_HOME="$legacy_data" \
    XDG_CONFIG_HOME="$legacy_config" XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_STUB_LOG="$legacy_log" PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
if ! printf '%s\n' "$legacy_output" | \
    grep -Fq 'Legacy OverCrow artifacts: 4 exact artifact(s) detected'; then
    printf '%s\n' 'diagnostic omitted the exact historical Lua artifact' >&2
    exit 1
fi
assert_tree_unchanged "$legacy_tree_before" "$legacy_home" "$legacy_data" "$legacy_config"
assert_read_only_log "$legacy_log"
assert_bounded_probe_pairs "$legacy_log"

# Legacy byte identity includes trailing line endings; an extra blank line must
# not be normalized away by shell command substitution.
printf '\n' >> "$legacy_config/hypr/overcrow.lua"
trailing_blank_output=$(
    HOME="$legacy_home" XDG_DATA_HOME="$legacy_data" \
    XDG_CONFIG_HOME="$legacy_config" XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_STUB_LOG="$legacy_log" PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
if printf '%s\n' "$trailing_blank_output" | \
    grep -Fq 'Legacy OverCrow artifacts: 4 exact artifact(s) detected'; then
    printf '%s\n' 'diagnostic normalized trailing blank lines in an exact artifact' >&2
    exit 1
fi
sed "s|@OVERCROWCTL@|$legacy_home/.local/bin/overcrowctl|g" \
    "$repo_root/tests/fixtures/legacy-overcrow.lua.in" \
    > "$legacy_config/hypr/overcrow.lua"

# Shell variables cannot represent NUL. The diagnostic must fail closed rather
# than silently drop an embedded byte and recover the reviewed fingerprint.
printf '\000' >> "$legacy_config/hypr/overcrow.lua"
nul_output=$(
    HOME="$legacy_home" XDG_DATA_HOME="$legacy_data" \
    XDG_CONFIG_HOME="$legacy_config" XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_STUB_LOG="$legacy_log" PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh" 2> "$tmpdir/legacy-nul.err"
)
if printf '%s\n' "$nul_output" | \
    grep -Fq 'Legacy OverCrow artifacts: 4 exact artifact(s) detected'; then
    printf '%s\n' 'diagnostic discarded an embedded NUL in an exact artifact' >&2
    exit 1
fi
sed "s|@OVERCROWCTL@|$legacy_home/.local/bin/overcrowctl|g" \
    "$repo_root/tests/fixtures/legacy-overcrow.lua.in" \
    > "$legacy_config/hypr/overcrow.lua"

huge_legacy_log="$tmpdir/huge-legacy.log"
: > "$huge_legacy_log"
/usr/bin/head -c 1048576 /dev/zero | /usr/bin/tr '\000' x \
    > "$legacy_config/hypr/hyprland.lua"
huge_legacy_output=$(
    /usr/bin/timeout 5s env \
        HOME="$legacy_home" XDG_DATA_HOME="$legacy_data" \
        XDG_CONFIG_HOME="$legacy_config" XDG_SESSION_TYPE=x11 \
        OVERCROW_DIAG_STUB_LOG="$huge_legacy_log" PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
if printf '%s\n' "$huge_legacy_output" | \
    grep -Fq 'Legacy OverCrow artifacts: 4 exact artifact(s) detected'; then
    printf '%s\n' 'diagnostic classified an oversized one-line Lua main as exact' >&2
    exit 1
fi
assert_read_only_log "$huge_legacy_log"
assert_bounded_probe_pairs "$huge_legacy_log"

# The exact legacy decision must use the bytes captured before the path is
# replaced; reopening the original path would observe the modified replacement.
printf '%s\n' \
    '-- BEGIN OVERCROW MANAGED' \
    'require("overcrow")' \
    '-- END OVERCROW MANAGED' \
    > "$legacy_config/hypr/hyprland.lua"
sed "s|@OVERCROWCTL@|$legacy_home/.local/bin/overcrowctl|g" \
    "$repo_root/tests/fixtures/legacy-overcrow.lua.in" \
    > "$legacy_config/hypr/overcrow.lua"
printf '%s\n' '# replacement after capture' \
    > "$tmpdir/legacy-lua-replacement"
: > "$tmpdir/legacy-swap.log"
legacy_swap_output=$(
    HOME="$legacy_home" XDG_DATA_HOME="$legacy_data" \
    XDG_CONFIG_HOME="$legacy_config" XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_SWAP_PATH="$legacy_config/hypr/overcrow.lua" \
    OVERCROW_DIAG_SWAP_REPLACEMENT="$tmpdir/legacy-lua-replacement" \
    OVERCROW_DIAG_STUB_LOG="$tmpdir/legacy-swap.log" PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$legacy_swap_output" | \
    grep -Fq 'Legacy OverCrow artifacts: 4 exact artifact(s) detected'

# Exercise the post-capture per-file bound without a production test seam. The
# generated copy simulates growth after descriptor validation by allowing head
# to return limit+1 bytes successfully under the larger global probe cap.
capture_leak_tmp="$tmpdir/capture-leak-tmp"
generated_diagnose="$tmpdir/generated-diagnose.sh"
mkdir -p "$capture_leak_tmp"
# shellcheck disable=SC2016 # Match literal variables in the generated script.
/usr/bin/sed \
    's/\[ "$descriptor_size" -le "$limit" \] || exit 2; //' \
    "$repo_root/scripts/diagnose.sh" > "$generated_diagnose"
chmod 0755 "$generated_diagnose"
if /usr/bin/cmp -s -- "$repo_root/scripts/diagnose.sh" "$generated_diagnose"; then
    printf '%s\n' 'generated diagnostic did not force the per-file oversize branch' >&2
    exit 1
fi
/usr/bin/head -c 65537 /dev/zero | /usr/bin/tr '\000' x \
    > "$legacy_config/hypr/overcrow.lua"
: > "$tmpdir/capture-leak.log"
capture_leak_output=$(
    HOME="$legacy_home" XDG_DATA_HOME="$legacy_data" \
    XDG_CONFIG_HOME="$legacy_config" XDG_SESSION_TYPE=x11 \
    TMPDIR="$capture_leak_tmp" \
    OVERCROW_DIAG_STUB_LOG="$tmpdir/capture-leak.log" PATH="$stub_bin" \
        "$generated_diagnose"
)
if printf '%s\n' "$capture_leak_output" | \
    grep -Fq 'Legacy OverCrow artifacts: 4 exact artifact(s) detected'; then
    printf '%s\n' 'per-file oversized capture was classified as exact' >&2
    exit 1
fi
if /usr/bin/find "$capture_leak_tmp" -maxdepth 1 \
    -name 'overcrow-diagnose.*' -print -quit | grep -q .; then
    printf '%s\n' 'diagnostic leaked a rejected private capture' >&2
    exit 1
fi

# Arbitrary fallback FIFOs must never block the read-only diagnostic.
fifo_home="$tmpdir/fifo-home"
fifo_data="$tmpdir/fifo-data"
fifo_config="$tmpdir/fifo-config"
mkdir -p "$fifo_home" "$fifo_data" "$fifo_config/hypr"
mkfifo "$fifo_config/hypr/hyprland.conf" "$fifo_config/kwinrc"
: > "$tmpdir/fifo.log"
set +e
fifo_output=$(
    /usr/bin/timeout 4s env \
        HOME="$fifo_home" XDG_DATA_HOME="$fifo_data" \
        XDG_CONFIG_HOME="$fifo_config" XDG_CURRENT_DESKTOP=Hyprland \
        OVERCROW_DIAG_FAIL_TARGETS=kreadconfig6 \
        OVERCROW_DIAG_STUB_LOG="$tmpdir/fifo.log" PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
fifo_status=$?
set -e
[ "$fifo_status" -eq 0 ] || {
    printf '%s\n' "diagnostic blocked or failed on fallback FIFOs: $fifo_status" >&2
    exit 1
}
printf '%s\n' "$fifo_output" | \
    grep -Fq 'Hyprland OverCrow config: incomplete or unlinked'
printf '%s\n' "$fifo_output" | \
    grep -Fq 'KWin package enabled: unavailable'

# A huge one-line fallback file is rejected within the outer bound.
oversized_fallback_config="$tmpdir/oversized-fallback-config"
mkdir -p "$oversized_fallback_config/hypr"
/usr/bin/head -c 33554432 /dev/zero | /usr/bin/tr '\000' x \
    > "$oversized_fallback_config/hypr/hyprland.conf"
: > "$tmpdir/oversized-fallback.log"
set +e
oversized_fallback_output=$(
    /usr/bin/timeout 4s env \
        HOME="$tmpdir/oversized-fallback-home" \
        XDG_DATA_HOME="$tmpdir/oversized-fallback-data" \
        XDG_CONFIG_HOME="$oversized_fallback_config" \
        XDG_CURRENT_DESKTOP=Hyprland \
        OVERCROW_DIAG_STUB_LOG="$tmpdir/oversized-fallback.log" PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
oversized_fallback_status=$?
set -e
[ "$oversized_fallback_status" -eq 0 ] || {
    printf '%s\n' "diagnostic exceeded fallback byte/time policy: $oversized_fallback_status" >&2
    exit 1
}
printf '%s\n' "$oversized_fallback_output" | \
    grep -Fq 'Hyprland OverCrow config: incomplete or unlinked'

minimal_bin="$tmpdir/minimal-bin"
mkdir -p "$minimal_bin"
ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" "$minimal_bin/timeout"
absent_output=$(
    env -i \
        HOME="$tmpdir/absent-home" \
        XDG_DATA_HOME="$tmpdir/absent-data" \
        XDG_CONFIG_HOME="$tmpdir/absent-config" \
        OVERCROW_DIAG_STUB_LOG="$tmpdir/absent.log" \
        PATH="$minimal_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)

printf '%s\n' "$absent_output" | grep -Fq "XDG_SESSION_TYPE: unavailable"
printf '%s\n' "$absent_output" | grep -Fq "D-Bus io.github.overcrow.Core1 owner: unavailable (busctl/gdbus not found)"
printf '%s\n' "$absent_output" | grep -Fq "overcrow-core.service: unavailable (systemctl not found)"
printf '%s\n' "$absent_output" | grep -Fq "overcrow-core process: skipped: current UID unavailable (id not found)"
printf '%s\n' "$absent_output" | grep -Fq "overcrow-overlay process: skipped: current UID unavailable (id not found)"
printf '%s\n' "$absent_output" | grep -Fq 'Lifecycle settings: missing (safe defaults)'
printf '%s\n' "$absent_output" | grep -Fq 'OverCrow master state: disabled'
printf '%s\n' "$absent_output" | grep -Fq 'Selected games: 0'
printf '%s\n' "$absent_output" | grep -Fq 'Shortcut availability: unavailable (Core ownership unknown)'
printf '%s\n' "$absent_output" | grep -Fq 'Legacy OverCrow artifacts: none'
printf '%s\n' "$absent_output" | grep -Fq "KWin package io.github.overcrow.kwin: unavailable (tool and readable metadata absent)"
printf '%s\n' "$absent_output" | grep -Fq "KWin package enabled: unavailable (tool and readable kwinrc absent)"
printf '%s\n' "$absent_output" | grep -Fq "Wayland globals: skipped (not a Wayland session)"
printf '%s\n' "$absent_output" | grep -Fq "X11 EWMH: skipped (DISPLAY unavailable)"

no_timeout_home="$tmpdir/no-timeout-home"
no_timeout_data="$tmpdir/no-timeout-data"
no_timeout_config="$tmpdir/no-timeout-config"
no_timeout_bin="$tmpdir/no-timeout-bin"
no_timeout_log="$tmpdir/no-timeout.log"
mkdir -p "$no_timeout_home" "$no_timeout_data" "$no_timeout_config" "$no_timeout_bin"
printf '%s\n' no-timeout-home-canary > "$no_timeout_home/canary"
printf '%s\n' no-timeout-data-canary > "$no_timeout_data/canary"
printf '%s\n' no-timeout-config-canary > "$no_timeout_config/canary"
: > "$no_timeout_log"
no_timeout_tree_before=$(snapshot_tree "$no_timeout_home" "$no_timeout_data" "$no_timeout_config")
for command_name in \
    id busctl gdbus systemctl pgrep kpackagetool6 kreadconfig6 wayland-info xprop
do
    ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" \
        "$no_timeout_bin/$command_name"
done
no_timeout_output=$(
    HOME="$no_timeout_home" \
    XDG_DATA_HOME="$no_timeout_data" \
    XDG_CONFIG_HOME="$no_timeout_config" \
    XDG_SESSION_TYPE=wayland \
    DISPLAY=:97 \
    OVERCROW_DIAG_STUB_LOG="$no_timeout_log" \
    PATH="$no_timeout_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)

printf '%s\n' "$no_timeout_output" | grep -Fq "D-Bus io.github.overcrow.Core1 owner: skipped: timeout unavailable"
printf '%s\n' "$no_timeout_output" | grep -Fq "overcrow-core.service: skipped: timeout unavailable"
printf '%s\n' "$no_timeout_output" | grep -Fq "overcrow-core process: skipped: timeout unavailable"
printf '%s\n' "$no_timeout_output" | grep -Fq "overcrow-overlay process: skipped: timeout unavailable"
printf '%s\n' "$no_timeout_output" | grep -Fq 'Lifecycle settings: missing (safe defaults)'
printf '%s\n' "$no_timeout_output" | grep -Fq 'Shortcut availability: skipped: timeout unavailable'
printf '%s\n' "$no_timeout_output" | grep -Fq "KWin package io.github.overcrow.kwin: skipped: timeout unavailable"
printf '%s\n' "$no_timeout_output" | grep -Fq "KWin package enabled: skipped: timeout unavailable"
printf '%s\n' "$no_timeout_output" | grep -Fq "Wayland globals: skipped: timeout unavailable"
printf '%s\n' "$no_timeout_output" | grep -Fq "X11 EWMH: skipped: timeout unavailable"
[ ! -s "$no_timeout_log" ]
assert_read_only_log "$no_timeout_log"
assert_bounded_probe_pairs "$no_timeout_log"
assert_tree_unchanged "$no_timeout_tree_before" \
    "$no_timeout_home" "$no_timeout_data" "$no_timeout_config"

hang_log="$tmpdir/hang.log"
: > "$hang_log"
bounded_timeout_output=$(
    HOME="$home" \
    XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" \
    XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_HANG_TARGETS=busctl,systemctl,pgrep,kpackagetool6,kreadconfig6 \
    OVERCROW_DIAG_STUB_LOG="$hang_log" \
    PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$bounded_timeout_output" | grep -Fq "D-Bus io.github.overcrow.Core1 owner: timed out"
printf '%s\n' "$bounded_timeout_output" | grep -Fq "overcrow-core.service: timed out"
printf '%s\n' "$bounded_timeout_output" | grep -Fq "overcrow-core process: timed out"
printf '%s\n' "$bounded_timeout_output" | grep -Fq "overcrow-overlay process: timed out"
printf '%s\n' "$bounded_timeout_output" | grep -Fq "KWin package io.github.overcrow.kwin: timed out"
printf '%s\n' "$bounded_timeout_output" | grep -Fq "KWin package enabled: timed out"

kill_status_output=$(
    HOME="$home" \
    XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" \
    XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_KILL_TARGETS=systemctl \
    OVERCROW_DIAG_STUB_LOG="$hang_log" \
    PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh" 2> "$tmpdir/kill-status.err"
)
printf '%s\n' "$kill_status_output" | grep -Fq "overcrow-core.service: timed out"
if [ -s "$tmpdir/kill-status.err" ]; then
    printf '%s\n' 'diagnostic leaked a killed-probe shell message to stderr' >&2
    exit 1
fi

error_output=$(
    HOME="$home" \
    XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" \
    XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_FAIL_TARGETS=busctl,systemctl,pgrep,kpackagetool6,kreadconfig6 \
    OVERCROW_DIAG_STUB_LOG="$hang_log" \
    PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$error_output" | grep -Fq "D-Bus io.github.overcrow.Core1 owner: unavailable (session bus query failed)"
printf '%s\n' "$error_output" | grep -Fq "overcrow-core.service: unavailable (user manager query failed)"
printf '%s\n' "$error_output" | grep -Fq "overcrow-core process: unavailable (pgrep failed)"
printf '%s\n' "$error_output" | grep -Fq "overcrow-overlay process: unavailable (pgrep failed)"
printf '%s\n' "$error_output" | grep -Fq "KWin package io.github.overcrow.kwin: unavailable (kpackagetool6 query failed)"
printf '%s\n' "$error_output" | grep -Fq "KWin package enabled: unavailable (kreadconfig6 query failed)"

uid_timeout_log="$tmpdir/uid-timeout.log"
: > "$uid_timeout_log"
uid_timeout_output=$(
    HOME="$home" \
    XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" \
    XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_HANG_TARGETS=id \
    OVERCROW_DIAG_STUB_LOG="$uid_timeout_log" \
    PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$uid_timeout_output" | grep -Fq "overcrow-core process: skipped: current UID probe timed out"
printf '%s\n' "$uid_timeout_output" | grep -Fq "overcrow-overlay process: skipped: current UID probe timed out"
if grep -Fq "pgrep " "$uid_timeout_log"; then
    printf '%s\n' "diagnostic ran pgrep without a current UID" >&2
    exit 1
fi
assert_read_only_log "$uid_timeout_log"
assert_bounded_probe_pairs "$uid_timeout_log"

invalid_uid_log="$tmpdir/invalid-uid.log"
: > "$invalid_uid_log"
invalid_uid_output=$(
    HOME="$home" \
    XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" \
    XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_STUB_UID=not-a-uid \
    OVERCROW_DIAG_STUB_LOG="$invalid_uid_log" \
    PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$invalid_uid_output" | grep -Fq "overcrow-core process: skipped: current UID unavailable (id -u failed)"
printf '%s\n' "$invalid_uid_output" | grep -Fq "overcrow-overlay process: skipped: current UID unavailable (id -u failed)"
if grep -Fq "pgrep " "$invalid_uid_log"; then
    printf '%s\n' "diagnostic ran pgrep with an invalid current UID" >&2
    exit 1
fi
assert_read_only_log "$invalid_uid_log"
assert_bounded_probe_pairs "$invalid_uid_log"

gdbus_bin="$tmpdir/gdbus-bin"
gdbus_log="$tmpdir/gdbus.log"
mkdir -p "$gdbus_bin"
: > "$gdbus_log"
ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" "$gdbus_bin/timeout"
ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" "$gdbus_bin/gdbus"
gdbus_timeout_output=$(
    HOME="$home" \
    XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" \
    XDG_SESSION_TYPE=x11 \
    OVERCROW_DIAG_HANG_TARGETS=gdbus \
    OVERCROW_DIAG_STUB_LOG="$gdbus_log" \
    PATH="$gdbus_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$gdbus_timeout_output" | grep -Fq "D-Bus io.github.overcrow.Core1 owner: timed out"
assert_read_only_log "$gdbus_log"
assert_bounded_probe_pairs "$gdbus_log"

wayland_timeout_output=$(
    HOME="$home" \
    XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" \
    XDG_SESSION_TYPE=wayland \
    DISPLAY=:96 \
    OVERCROW_DIAG_HANG_TARGET=wayland-info \
    OVERCROW_DIAG_STUB_LOG="$hang_log" \
    PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$wayland_timeout_output" | grep -Fq "Wayland globals: timed out"

xprop_timeout_output=$(
    HOME="$home" \
    XDG_DATA_HOME="$data_home" \
    XDG_CONFIG_HOME="$config_home" \
    XDG_SESSION_TYPE=wayland \
    DISPLAY=:95 \
    OVERCROW_DIAG_HANG_TARGET=xprop \
    OVERCROW_DIAG_STUB_LOG="$hang_log" \
    PATH="$stub_bin" \
        "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$xprop_timeout_output" | grep -Fq "X11 EWMH _NET_SUPPORTING_WM_CHECK: timed out"
printf '%s\n' "$xprop_timeout_output" | grep -Fq "X11 EWMH _NET_ACTIVE_WINDOW: timed out"
assert_tree_unchanged "$present_tree_before" "$home" "$data_home" "$config_home"
assert_read_only_log "$hang_log"
assert_bounded_probe_pairs "$hang_log"

real_timeout=$(command -v timeout)
real_timeout_bin="$tmpdir/real-timeout-bin"
real_timeout_log="$tmpdir/real-timeout.log"
mkdir -p "$real_timeout_bin"
: > "$real_timeout_log"
ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" "$real_timeout_bin/timeout"
ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" "$real_timeout_bin/busctl"
set +e
ignore_term_output=$(
    "$real_timeout" --signal=KILL 7s \
        /usr/bin/env \
        HOME="$home" \
        XDG_DATA_HOME="$data_home" \
        XDG_CONFIG_HOME="$config_home" \
        XDG_SESSION_TYPE=x11 \
        OVERCROW_DIAG_IGNORE_TERM_TARGETS=busctl \
        OVERCROW_DIAG_REAL_TIMEOUT="$real_timeout" \
        OVERCROW_DIAG_STUB_LOG="$real_timeout_log" \
        PATH="$real_timeout_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh" 2> "$tmpdir/ignore-term.err"
)
ignore_term_status=$?
set -e
if [ "$ignore_term_status" -ne 0 ]; then
    printf '%s\n' "TERM-ignoring probe exceeded the diagnostic bound" >&2
    exit 1
fi
printf '%s\n' "$ignore_term_output" | grep -Fq "D-Bus io.github.overcrow.Core1 owner: timed out"
if [ -s "$tmpdir/ignore-term.err" ]; then
    printf '%s\n' 'diagnostic leaked a killed-probe shell message to stderr' >&2
    exit 1
fi
assert_read_only_log "$real_timeout_log"
assert_bounded_probe_pairs "$real_timeout_log"

fallback_home="$tmpdir/fallback-home"
fallback_data="$tmpdir/fallback-data"
fallback_config="$tmpdir/fallback-config"
fallback_bin="$tmpdir/fallback-bin"
fallback_log="$tmpdir/fallback.log"
mkdir -p "$fallback_home" \
    "$fallback_data/kwin/scripts/io.github.overcrow.kwin" \
    "$fallback_config" "$fallback_bin"
printf '%s\n' '{"KPlugin":{"Id":"io.github.overcrow.kwin"}}' \
    > "$fallback_data/kwin/scripts/io.github.overcrow.kwin/metadata.json"
printf '%s\n' '[Plugins]' 'io.github.overcrow.kwinEnabled=true' \
    > "$fallback_config/kwinrc"
: > "$fallback_log"
ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" "$fallback_bin/timeout"
ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" "$fallback_bin/xprop"

fallback_tree_before=$(snapshot_tree "$fallback_home" "$fallback_data" "$fallback_config")
fallback_output=$(
    HOME="$fallback_home" \
    XDG_DATA_HOME="$fallback_data" \
    XDG_CONFIG_HOME="$fallback_config" \
    XDG_SESSION_TYPE=x11 \
    DISPLAY=:98 \
    OVERCROW_DIAG_STUB_LOG="$fallback_log" \
    PATH="$fallback_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)

printf '%s\n' "$fallback_output" | grep -Fq "KWin package io.github.overcrow.kwin: installed (metadata file)"
printf '%s\n' "$fallback_output" | grep -Fq "KWin package enabled: yes (kwinrc)"
printf '%s\n' "$fallback_output" | grep -Fq "Wayland globals: skipped (not a Wayland session)"
assert_tree_unchanged "$fallback_tree_before" "$fallback_home" "$fallback_data" "$fallback_config"
assert_read_only_log "$fallback_log"
assert_bounded_probe_pairs "$fallback_log"

hypr_home="$tmpdir/hypr-home"
hypr_data="$tmpdir/hypr-data"
hypr_config="$tmpdir/hypr-config"
hypr_runtime="$tmpdir/hypr-runtime"
hypr_bin="$tmpdir/hypr-bin"
hypr_log="$tmpdir/hypr.log"
hypr_signature=fixture_123
hypr_instance="$hypr_runtime/hypr/$hypr_signature"
mkdir -p "$hypr_home" "$hypr_data" "$hypr_config/hypr" "$hypr_instance" "$hypr_bin"
: > "$hypr_log"
printf '%s\n' \
    '# user config' \
    '# BEGIN OVERCROW MANAGED' \
    "source = $hypr_config/hypr/overcrow.conf" \
    '# END OVERCROW MANAGED' \
    > "$hypr_config/hypr/hyprland.conf"
printf '%s\n' '# Managed by OverCrow. Changes are replaced by the installer.' \
    > "$hypr_config/hypr/overcrow.conf"
hypr_socket_state=absent
if /usr/bin/python3 -c \
    'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()' \
    "$hypr_instance/.socket.sock" 2>/dev/null && \
    /usr/bin/python3 -c \
    'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()' \
    "$hypr_instance/.socket2.sock" 2>/dev/null; then
    hypr_socket_state=present
fi
for command_name in id busctl systemctl pgrep timeout hyprctl wayland-info; do
    ln -s "$repo_root/tests/fixtures/diagnose-command-stub.sh" "$hypr_bin/$command_name"
done
ln -s /usr/bin/jq "$hypr_bin/jq"

hypr_tree_before=$(snapshot_tree "$hypr_home" "$hypr_data" "$hypr_config")
hypr_output=$(
    HOME="$hypr_home" \
    XDG_DATA_HOME="$hypr_data" \
    XDG_CONFIG_HOME="$hypr_config" \
    XDG_RUNTIME_DIR="$hypr_runtime" \
    HYPRLAND_INSTANCE_SIGNATURE="$hypr_signature" \
    XDG_SESSION_TYPE=wayland \
    XDG_CURRENT_DESKTOP=Hyprland \
    DESKTOP_SESSION=hyprland-uwsm \
    OVERCROW_DIAG_STUB_LOG="$hypr_log" \
    PATH="$hypr_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$hypr_output" | grep -Fq 'Hyprland version: 0.55.2'
printf '%s\n' "$hypr_output" | grep -Fq "Hyprland command socket: $hypr_socket_state"
printf '%s\n' "$hypr_output" | grep -Fq "Hyprland event socket: $hypr_socket_state"
printf '%s\n' "$hypr_output" | grep -Fq 'Hyprland bridge service: active'
printf '%s\n' "$hypr_output" | grep -Fq 'Hyprland OverCrow config: linked'
printf '%s\n' "$hypr_output" | grep -Fq 'Hyprland config errors: none'
printf '%s\n' "$hypr_output" | grep -Fq 'Hyprland overlay windows: 1'
printf '%s\n' "$hypr_output" | \
    grep -Fq 'Hyprland OverCrow runtime shortcut: active (SUPER + ALT + O)'
if printf '%s\n' "$hypr_output" | grep -Fq 'private game title'; then
    printf '%s\n' 'Hyprland diagnostic leaked client JSON' >&2
    exit 1
fi

legacy_shortcut_output=$(
    HOME="$hypr_home" \
    XDG_CONFIG_HOME="$hypr_config" \
    XDG_RUNTIME_DIR="$hypr_runtime" \
    HYPRLAND_INSTANCE_SIGNATURE="$hypr_signature" \
    XDG_CURRENT_DESKTOP=Hyprland \
    OVERCROW_DIAG_SHORTCUT_STATE=legacy \
    OVERCROW_DIAG_STUB_LOG="$hypr_log" \
    PATH="$hypr_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$legacy_shortcut_output" | \
    grep -Fq 'Hyprland OverCrow runtime shortcut: active (SUPER + ALT + O)'

inactive_shortcut_output=$(
    HOME="$hypr_home" \
    XDG_CONFIG_HOME="$hypr_config" \
    XDG_RUNTIME_DIR="$hypr_runtime" \
    HYPRLAND_INSTANCE_SIGNATURE="$hypr_signature" \
    XDG_CURRENT_DESKTOP=Hyprland \
    OVERCROW_DIAG_SHORTCUT_STATE=inactive \
    OVERCROW_DIAG_STUB_LOG="$hypr_log" \
    PATH="$hypr_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$inactive_shortcut_output" | \
    grep -Fq 'Hyprland OverCrow runtime shortcut: inactive'

conflicting_shortcut_output=$(
    HOME="$hypr_home" \
    XDG_CONFIG_HOME="$hypr_config" \
    XDG_RUNTIME_DIR="$hypr_runtime" \
    HYPRLAND_INSTANCE_SIGNATURE="$hypr_signature" \
    XDG_CURRENT_DESKTOP=Hyprland \
    OVERCROW_DIAG_SHORTCUT_STATE=conflict \
    OVERCROW_DIAG_STUB_LOG="$hypr_log" \
    PATH="$hypr_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$conflicting_shortcut_output" | \
    grep -Fq 'Hyprland OverCrow runtime shortcut: conflict on SUPER + ALT + O'

assert_tree_unchanged "$hypr_tree_before" "$hypr_home" "$hypr_data" "$hypr_config"
assert_read_only_log "$hypr_log"
assert_bounded_probe_pairs "$hypr_log"

malformed_hypr_output=$(
    HOME="$hypr_home" \
    XDG_CONFIG_HOME="$hypr_config" \
    XDG_RUNTIME_DIR="$hypr_runtime" \
    HYPRLAND_INSTANCE_SIGNATURE="$hypr_signature" \
    XDG_CURRENT_DESKTOP=Hyprland \
    OVERCROW_DIAG_MALFORMED_HYPRLAND=1 \
    OVERCROW_DIAG_STUB_LOG="$hypr_log" \
    PATH="$hypr_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$malformed_hypr_output" | grep -Fq 'Hyprland overlay windows: malformed response'

minified_hypr_output=$(
    HOME="$hypr_home" \
    XDG_CONFIG_HOME="$hypr_config" \
    XDG_RUNTIME_DIR="$hypr_runtime" \
    HYPRLAND_INSTANCE_SIGNATURE="$hypr_signature" \
    XDG_CURRENT_DESKTOP=Hyprland \
    OVERCROW_DIAG_MINIFIED_HYPRLAND=1 \
    OVERCROW_DIAG_STUB_LOG="$hypr_log" \
    PATH="$hypr_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$minified_hypr_output" | grep -Fq 'Hyprland overlay windows: 1'

oversized_hypr_output=$(
    HOME="$hypr_home" \
    XDG_CONFIG_HOME="$hypr_config" \
    XDG_RUNTIME_DIR="$hypr_runtime" \
    HYPRLAND_INSTANCE_SIGNATURE="$hypr_signature" \
    XDG_CURRENT_DESKTOP=Hyprland \
    OVERCROW_DIAG_OVERSIZED_HYPRLAND=1 \
    OVERCROW_DIAG_OVERSIZED_COMPLETED_MARKER="$tmpdir/oversized-producer-completed" \
    OVERCROW_DIAG_STUB_LOG="$hypr_log" \
    PATH="$hypr_bin" \
        /bin/sh "$repo_root/scripts/diagnose.sh"
)
printf '%s\n' "$oversized_hypr_output" | \
    grep -Fq 'Hyprland overlay windows: unavailable (response too large)'
[ ! -e "$tmpdir/oversized-producer-completed" ]

printf '%s\n' "Diagnostic smoke test passed"
