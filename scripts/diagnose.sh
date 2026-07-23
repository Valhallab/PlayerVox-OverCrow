#!/bin/sh
# shellcheck disable=SC2016
set -u

dbus_name=io.github.overcrow.Core1
kwin_id=io.github.overcrow.kwin
probe_timeout=3s
probe_kill_after=1s
probe_max_bytes=4194304
probe_capture_bytes=$((probe_max_bytes + 1))

if command -v timeout >/dev/null 2>&1; then
    diag_timeout_available=true
else
    diag_timeout_available=false
fi

diag_probe_output=
diag_probe_status=
diag_probe_exit=
diag_probe_capture_file=
diag_probe_preserve_file=false
diag_dbus_owner_state=unknown

prepare_probe_capture() {
    diag_probe_file=$(/usr/bin/mktemp "${TMPDIR:-/tmp}/overcrow-diagnose.XXXXXX") || {
        diag_probe_status=error
        return 1
    }
    diag_probe_fifo="${diag_probe_file}.fifo"
    if ! /usr/bin/mkfifo -- "$diag_probe_fifo"; then
        /usr/bin/rm -f -- "$diag_probe_file"
        diag_probe_status=error
        return 1
    fi
}

collect_probe_capture() {
    diag_probe_reader_exit=0
    /usr/bin/head -c "$probe_capture_bytes" < "$diag_probe_fifo" \
        > "$diag_probe_file" || diag_probe_reader_exit=$?
    wait "$diag_probe_pid"
    diag_probe_exit=$?
    /usr/bin/rm -f -- "$diag_probe_fifo"
    if [ "$diag_probe_reader_exit" -ne 0 ]; then
        /usr/bin/rm -f -- "$diag_probe_file"
        diag_probe_status=error
        return
    fi
    finish_probe_capture
}

finish_probe_capture() {
    diag_probe_size=$(/usr/bin/wc -c < "$diag_probe_file" 2>/dev/null) || {
        /usr/bin/rm -f -- "$diag_probe_file"
        diag_probe_status=error
        return
    }
    case $diag_probe_size in
        ''|*[!0-9]*)
            /usr/bin/rm -f -- "$diag_probe_file"
            diag_probe_status=error
            return
            ;;
    esac
    if [ "$diag_probe_size" -gt "$probe_max_bytes" ]; then
        /usr/bin/rm -f -- "$diag_probe_file"
        diag_probe_output=
        diag_probe_status=oversized
        return
    fi
    if [ "$diag_probe_preserve_file" = true ] && [ "$diag_probe_exit" = 0 ]; then
        diag_probe_capture_file=$diag_probe_file
        diag_probe_output=
    else
        diag_probe_output=$(/usr/bin/cat -- "$diag_probe_file")
        /usr/bin/rm -f -- "$diag_probe_file"
    fi
    case "$diag_probe_exit" in
        0)
            diag_probe_status=ok
            ;;
        124|137)
            diag_probe_status=timed_out
            ;;
        *)
            diag_probe_status=error
            ;;
    esac
}

run_probe() {
    diag_probe_command=$1
    shift
    diag_probe_output=
    diag_probe_exit=

    if [ "$diag_timeout_available" != true ]; then
        diag_probe_status=timeout_unavailable
        return
    fi
    if ! command -v "$diag_probe_command" >/dev/null 2>&1; then
        diag_probe_status=not_found
        return
    fi

    prepare_probe_capture || return
    {
        timeout \
            --signal=TERM \
            --kill-after="$probe_kill_after" \
            "$probe_timeout" \
            "$diag_probe_command" "$@" > "$diag_probe_fifo" 2>/dev/null
    } 2>/dev/null &
    diag_probe_pid=$!
    collect_probe_capture
}

run_probe_to_file() {
    diag_probe_command=$1
    shift
    diag_probe_output=
    diag_probe_exit=
    diag_probe_capture_file=

    if [ "$diag_timeout_available" != true ]; then
        diag_probe_status=timeout_unavailable
        return
    fi
    if ! command -v "$diag_probe_command" >/dev/null 2>&1; then
        diag_probe_status=not_found
        return
    fi

    prepare_probe_capture || return
    diag_probe_preserve_file=true
    {
        timeout \
            --signal=TERM \
            --kill-after="$probe_kill_after" \
            "$probe_timeout" \
            "$diag_probe_command" "$@" > "$diag_probe_fifo" 2>/dev/null
    } 2>/dev/null &
    diag_probe_pid=$!
    collect_probe_capture
    diag_probe_preserve_file=false
}

run_probe_with_input() {
    diag_probe_input=$1
    diag_probe_command=$2
    shift 2
    diag_probe_output=
    diag_probe_exit=

    if [ "$diag_timeout_available" != true ]; then
        diag_probe_status=timeout_unavailable
        return
    fi
    if ! command -v "$diag_probe_command" >/dev/null 2>&1; then
        diag_probe_status=not_found
        return
    fi

    prepare_probe_capture || return
    {
        printf '%s' "$diag_probe_input" | timeout \
            --signal=TERM \
            --kill-after="$probe_kill_after" \
            "$probe_timeout" \
            "$diag_probe_command" "$@" > "$diag_probe_fifo" 2>/dev/null
    } 2>/dev/null &
    diag_probe_pid=$!
    collect_probe_capture
}

print_environment_value() {
    diag_label=$1
    diag_value=$2
    if [ -n "$diag_value" ]; then
        printf '%s: %s\n' "$diag_label" "$diag_value"
    else
        printf '%s: unavailable\n' "$diag_label"
    fi
}

report_dbus_owner() {
    if [ "$diag_timeout_available" != true ]; then
        printf '%s\n' "D-Bus $dbus_name owner: skipped: timeout unavailable"
        return
    fi

    diag_dbus_attempted=false
    diag_dbus_timed_out=false

    if command -v busctl >/dev/null 2>&1; then
        diag_dbus_attempted=true
        run_probe busctl --user call \
            org.freedesktop.DBus \
            /org/freedesktop/DBus \
            org.freedesktop.DBus \
            NameHasOwner \
            s \
            "$dbus_name"
        case "$diag_probe_status" in
            ok)
                case "$diag_probe_output" in
                "b true")
                    diag_dbus_owner_state=yes
                    printf '%s\n' "D-Bus $dbus_name owner: yes"
                    return
                    ;;
                "b false")
                    diag_dbus_owner_state=no
                    printf '%s\n' "D-Bus $dbus_name owner: no"
                    return
                    ;;
                esac
                ;;
            timed_out)
                diag_dbus_timed_out=true
                ;;
        esac
    fi

    if command -v gdbus >/dev/null 2>&1; then
        diag_dbus_attempted=true
        run_probe gdbus call \
            --session \
            --dest org.freedesktop.DBus \
            --object-path /org/freedesktop/DBus \
            --method org.freedesktop.DBus.NameHasOwner \
            "$dbus_name"
        case "$diag_probe_status" in
            ok)
                case "$diag_probe_output" in
                "(true,)")
                    diag_dbus_owner_state=yes
                    printf '%s\n' "D-Bus $dbus_name owner: yes"
                    return
                    ;;
                "(false,)")
                    diag_dbus_owner_state=no
                    printf '%s\n' "D-Bus $dbus_name owner: no"
                    return
                    ;;
                esac
                ;;
            timed_out)
                diag_dbus_timed_out=true
                ;;
        esac
    fi

    if [ "$diag_dbus_timed_out" = true ]; then
        printf '%s\n' "D-Bus $dbus_name owner: timed out"
    elif [ "$diag_dbus_attempted" = true ]; then
        printf '%s\n' "D-Bus $dbus_name owner: unavailable (session bus query failed)"
    else
        printf '%s\n' "D-Bus $dbus_name owner: unavailable (busctl/gdbus not found)"
    fi
}

report_lifecycle_settings() {
    case ${XDG_CONFIG_HOME:-} in
    /*)
        diag_settings_path=$XDG_CONFIG_HOME/overcrow/settings.json
        ;;
    *)
        case ${HOME:-} in
        /*)
        diag_settings_path=$HOME/.config/overcrow/settings.json
            ;;
        *)
        printf '%s\n' 'Lifecycle settings: unavailable (config home unknown)'
        printf '%s\n' 'OverCrow master state: unavailable'
        printf '%s\n' 'Selected games: unavailable'
        return
            ;;
        esac
        ;;
    esac

    if [ ! -e "$diag_settings_path" ] && [ ! -L "$diag_settings_path" ]; then
        printf '%s\n' 'Lifecycle settings: missing (safe defaults)'
        printf '%s\n' 'OverCrow master state: disabled'
        printf '%s\n' 'Selected games: 0'
        return
    fi
    diag_settings_helper=
    case ${HOME:-} in
        /*)
            diag_local_settings_helper=$HOME/.local/bin/overcrow-control
            if [ -f "$diag_local_settings_helper" ] && \
                [ -x "$diag_local_settings_helper" ] && \
                [ ! -L "$diag_local_settings_helper" ]; then
                diag_settings_helper=$diag_local_settings_helper
            fi
            ;;
    esac
    if [ -z "$diag_settings_helper" ] && \
        [ -f /usr/bin/overcrow-control ] && \
        [ -x /usr/bin/overcrow-control ] && \
        [ ! -L /usr/bin/overcrow-control ]; then
        diag_settings_helper=/usr/bin/overcrow-control
    fi
    if [ -z "$diag_settings_helper" ]; then
        printf '%s\n' 'Lifecycle settings: unavailable (safe validator missing)'
        printf '%s\n' 'OverCrow master state: unavailable'
        printf '%s\n' 'Selected games: unavailable'
        return
    fi

    run_probe "$diag_settings_helper" --overcrow-diagnose-settings-v1

    case $diag_probe_status:$diag_probe_output in
        ok:valid\|enabled\|*)
            diag_settings_count=${diag_probe_output#valid|enabled|}
            diag_settings_enabled=enabled
            ;;
        ok:valid\|disabled\|*)
            diag_settings_count=${diag_probe_output#valid|disabled|}
            diag_settings_enabled=disabled
            ;;
        ok:invalid\|disabled\|0)
            diag_settings_count=
            diag_settings_enabled=
            ;;
        ok:missing\|disabled\|0)
            printf '%s\n' 'Lifecycle settings: missing (safe defaults)'
            printf '%s\n' 'OverCrow master state: disabled'
            printf '%s\n' 'Selected games: 0'
            return
            ;;
        ok:unavailable\|unavailable\|unavailable)
            printf '%s\n' 'Lifecycle settings: unavailable (config home unknown)'
            printf '%s\n' 'OverCrow master state: unavailable'
            printf '%s\n' 'Selected games: unavailable'
            return
            ;;
        timeout_unavailable:*)
            printf '%s\n' 'Lifecycle settings: skipped: timeout unavailable'
            printf '%s\n' 'OverCrow master state: unavailable'
            printf '%s\n' 'Selected games: unavailable'
            return
            ;;
        not_found:*)
            printf '%s\n' 'Lifecycle settings: unavailable (safe validator missing)'
            printf '%s\n' 'OverCrow master state: unavailable'
            printf '%s\n' 'Selected games: unavailable'
            return
            ;;
        timed_out:*)
            printf '%s\n' 'Lifecycle settings: validation timed out'
            printf '%s\n' 'OverCrow master state: unavailable'
            printf '%s\n' 'Selected games: unavailable'
            return
            ;;
        *)
            diag_settings_count=
            diag_settings_enabled=
            ;;
    esac
    case $diag_settings_count in
        ''|*[!0-9]*)
            printf '%s\n' 'Lifecycle settings: invalid or unsafe (runtime defaults to disabled)'
            printf '%s\n' 'OverCrow master state: disabled'
            printf '%s\n' 'Selected games: 0'
            ;;
        *)
            printf '%s\n' 'Lifecycle settings: valid'
            printf '%s\n' "OverCrow master state: $diag_settings_enabled"
            printf '%s\n' "Selected games: $diag_settings_count"
            ;;
    esac
}

report_shortcut_availability() {
    if [ "$diag_timeout_available" != true ]; then
        printf '%s\n' 'Shortcut availability: skipped: timeout unavailable'
        return
    fi
    if [ "$diag_dbus_owner_state" = no ]; then
        printf '%s\n' 'Shortcut availability: unavailable (Core is not running)'
        return
    fi
    if [ "$diag_dbus_owner_state" != yes ]; then
        printf '%s\n' 'Shortcut availability: unavailable (Core ownership unknown)'
        return
    fi
    run_probe busctl --user call \
        "$dbus_name" /io/github/overcrow/Core1 "$dbus_name" ShortcutAvailability
    case $diag_probe_status in
        timeout_unavailable)
            printf '%s\n' 'Shortcut availability: skipped: timeout unavailable'
            ;;
        not_found)
            printf '%s\n' 'Shortcut availability: unavailable (busctl not found)'
            ;;
        timed_out)
            printf '%s\n' 'Shortcut availability: timed out'
            ;;
        ok)
            case $diag_probe_output in
                's "available"') printf '%s\n' 'Shortcut availability: available' ;;
                's "binding"') printf '%s\n' 'Shortcut availability: binding' ;;
                's "disabled"') printf '%s\n' 'Shortcut availability: disabled' ;;
                's "unavailable: '*'"')
                    printf '%s\n' 'Shortcut availability: unavailable (details withheld)'
                    ;;
                *) printf '%s\n' 'Shortcut availability: unavailable (unrecognized response)' ;;
            esac
            ;;
        *) printf '%s\n' 'Shortcut availability: unavailable (Core query failed)' ;;
    esac
}

diagnostic_control_file_capture() {
    diag_control_file=$1
    diag_control_limit=$2
    diag_control_capture_file=
    run_probe_to_file /bin/sh -c \
        'path=$1; limit=$2; [ ! -L "$path" ] || exit 2; [ "$(/usr/bin/stat -c %F -- "$path" 2>/dev/null)" = "regular file" ] || exit 2; exec 3< "$path" || exit 2; [ ! -L "$path" ] || exit 2; path_identity=$(/usr/bin/stat -c %d:%i -- "$path" 2>/dev/null) || exit 2; descriptor_identity=$(/usr/bin/stat -Lc %d:%i -- /proc/self/fd/3 2>/dev/null) || exit 2; [ "$path_identity" = "$descriptor_identity" ] || exit 2; [ "$(/usr/bin/stat -Lc %F -- /proc/self/fd/3 2>/dev/null)" = "regular file" ] || exit 2; descriptor_size=$(/usr/bin/stat -Lc %s -- /proc/self/fd/3 2>/dev/null) || exit 2; case $descriptor_size in ""|*[!0-9]*) exit 2 ;; esac; [ "$descriptor_size" -le "$limit" ] || exit 2; /usr/bin/head -c "$((limit + 1))" <&3' \
        overcrow-diagnostic-capture "$diag_control_file" "$diag_control_limit"
    if [ "$diag_probe_status" != ok ] || \
        [ "$diag_probe_size" -gt "$diag_control_limit" ]; then
        remove_diagnostic_capture "$diag_probe_capture_file"
        diag_probe_capture_file=
        return 1
    fi
    diag_control_capture_file=$diag_probe_capture_file
}

remove_diagnostic_capture() {
    [ -z "$1" ] || /usr/bin/rm -f -- "$1"
}

legacy_hypr_fragment_exact() {
    diag_legacy_file=$1
    diag_legacy_binary_dir=$2
    diagnostic_control_file_capture "$diag_legacy_file" 65536 || return 1
    diag_legacy_fragment_capture=$diag_control_capture_file
    diag_expected_fragment=$(/usr/bin/mktemp \
        "${TMPDIR:-/tmp}/overcrow-legacy-fragment.XXXXXX") || {
        /usr/bin/rm -f -- "$diag_legacy_fragment_capture"
        return 1
    }
    if ! /usr/bin/printf '%s\n' \
        '# Managed by OverCrow. Changes are replaced by the installer.' \
        'exec-once = systemctl --user import-environment HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP WAYLAND_DISPLAY XDG_RUNTIME_DIR && systemctl --user restart overcrow-hyprland.service' \
        'windowrule = match:class ^(io\.github\.overcrow\.Overlay)$, float on, no_focus on, no_initial_focus on, no_follow_mouse on, focus_on_activate off, decorate off, no_anim on, no_blur on, no_dim on, no_shadow on, no_shortcuts_inhibit on, suppress_event maximize fullscreen' \
        'windowrule = match:tag overcrow-interactive, no_focus off, no_follow_mouse off' \
        'windowrule = match:tag overcrow-game-input-blocked, no_focus on' \
        "bind = SUPER ALT, O, exec, $diag_legacy_binary_dir/overcrowctl toggle" \
        > "$diag_expected_fragment"; then
        /usr/bin/rm -f -- "$diag_legacy_fragment_capture" "$diag_expected_fragment"
        return 1
    fi
    run_probe /usr/bin/cmp -s -- "$diag_legacy_fragment_capture" "$diag_expected_fragment"
    diag_legacy_fragment_exact_status=$diag_probe_status
    /usr/bin/rm -f -- "$diag_legacy_fragment_capture" "$diag_expected_fragment"
    [ "$diag_legacy_fragment_exact_status" = ok ]
}

legacy_hypr_lua_block_exact() {
    diag_legacy_lua_main=$1
    diagnostic_control_file_capture "$diag_legacy_lua_main" 262144 || return 1
    diag_legacy_lua_main_capture=$diag_control_capture_file
    run_probe /usr/bin/gawk \
        'previous_two == "-- BEGIN OVERCROW MANAGED" && previous_one == "require(\"overcrow\")" && $0 == "-- END OVERCROW MANAGED" { count++ } { previous_two = previous_one; previous_one = $0 } END { print count + 0 }' \
        "$diag_legacy_lua_main_capture"
    diag_legacy_lua_block_status=$diag_probe_status
    diag_legacy_lua_block_count=$diag_probe_output
    /usr/bin/rm -f -- "$diag_legacy_lua_main_capture"
    [ "$diag_legacy_lua_block_status" = ok ] && [ "$diag_legacy_lua_block_count" = 1 ]
}

legacy_hypr_lua_exact() {
    diag_legacy_lua_file=$1
    diag_legacy_lua_binary_dir=$2
    diagnostic_control_file_capture "$diag_legacy_lua_file" 65536 || return 1
    diag_legacy_lua_capture=$diag_control_capture_file
    diag_legacy_lua_expected=$(/usr/bin/mktemp \
        "${TMPDIR:-/tmp}/overcrow-legacy-lua.XXXXXX") || {
        /usr/bin/rm -f -- "$diag_legacy_lua_capture"
        return 1
    }
    if ! /usr/bin/printf '%s\n' \
        '-- Managed by OverCrow. Changes are replaced by the installer.' \
        'hl.on("hyprland.start", function()' \
        '    hl.exec_cmd("systemctl --user import-environment HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP WAYLAND_DISPLAY XDG_RUNTIME_DIR && systemctl --user restart overcrow-hyprland.service")' \
        'end)' \
        'hl.on("hyprland.shutdown", function()' \
        '    os.execute("systemctl --user stop overcrow-hyprland.service")' \
        'end)' \
        'hl.window_rule({' \
        '    name = "overcrow-overlay",' \
        '    match = { class = "^(io\\.github\\.overcrow\\.Overlay)$" },' \
        '    float = true,' \
        '    no_focus = true,' \
        '    no_initial_focus = true,' \
        '    no_follow_mouse = true,' \
        '    focus_on_activate = false,' \
        '    decorate = false,' \
        '    no_anim = true,' \
        '    no_blur = true,' \
        '    no_dim = true,' \
        '    no_shadow = true,' \
        '    no_shortcuts_inhibit = true,' \
        '    suppress_event = "maximize fullscreen",' \
        '})' \
        'hl.window_rule({' \
        '    name = "overcrow-overlay-interactive",' \
        '    match = { tag = "overcrow-interactive" },' \
        '    no_focus = false,' \
        '    no_follow_mouse = false,' \
        '})' \
        'hl.window_rule({' \
        '    name = "overcrow-game-input-blocked",' \
        '    match = { tag = "overcrow-game-input-blocked" },' \
        '    no_focus = true,' \
        '})' \
        "hl.bind(\"SUPER + ALT + O\", hl.dsp.exec_cmd(\"$diag_legacy_lua_binary_dir/overcrowctl toggle\"))" \
        > "$diag_legacy_lua_expected"; then
        /usr/bin/rm -f -- "$diag_legacy_lua_capture" "$diag_legacy_lua_expected"
        return 1
    fi
    run_probe /usr/bin/cmp -s -- "$diag_legacy_lua_capture" "$diag_legacy_lua_expected"
    /usr/bin/rm -f -- "$diag_legacy_lua_capture" "$diag_legacy_lua_expected"
    [ "$diag_probe_status" = ok ]
}

report_legacy_artifacts() {
    diag_legacy_home=${HOME:-}
    if [ -z "$diag_legacy_home" ]; then
        printf '%s\n' 'Legacy OverCrow artifacts: unavailable (HOME unknown)'
        return
    fi
    diag_legacy_data=${XDG_DATA_HOME:-$diag_legacy_home/.local/share}
    diag_legacy_config=${XDG_CONFIG_HOME:-$diag_legacy_home/.config}
    diag_legacy_units="$diag_legacy_data/systemd/user"
    diag_legacy_count=0

    for diag_legacy_unit in overcrow-core.service overcrow-overlay.service overcrow-hyprland.service; do
        for diag_legacy_target in default.target graphical-session.target; do
            diag_legacy_link="$diag_legacy_config/systemd/user/$diag_legacy_target.wants/$diag_legacy_unit"
            if [ -L "$diag_legacy_link" ]; then
                run_probe readlink -m -- "$diag_legacy_link"
                if [ "$diag_probe_status" = ok ] && \
                    [ "$diag_probe_output" = "$diag_legacy_units/$diag_legacy_unit" ]; then
                    diag_legacy_count=$((diag_legacy_count + 1))
                fi
            fi
        done
    done
    if legacy_hypr_fragment_exact "$diag_legacy_config/hypr/overcrow.conf" \
        "$diag_legacy_home/.local/bin"; then
        diag_legacy_count=$((diag_legacy_count + 1))
    fi
    if legacy_hypr_lua_block_exact "$diag_legacy_config/hypr/hyprland.lua" && \
        legacy_hypr_lua_exact "$diag_legacy_config/hypr/overcrow.lua" \
            "$diag_legacy_home/.local/bin"; then
        diag_legacy_count=$((diag_legacy_count + 1))
    fi

    diag_legacy_launcher="$diag_legacy_data/applications/io.github.overcrow.Overlay.desktop"
    if [ -f "$diag_legacy_launcher" ] && [ ! -L "$diag_legacy_launcher" ]; then
        run_probe sha256sum -- "$diag_legacy_launcher"
        case $diag_probe_status:$diag_probe_output in
            ok:2fa00167256bee8ee616e314a9c354801cd08eb1d693678cea24ec906e18aec8*)
                diag_legacy_count=$((diag_legacy_count + 1))
                ;;
        esac
    fi

    if [ "$diag_legacy_count" -eq 0 ]; then
        printf '%s\n' 'Legacy OverCrow artifacts: none'
    else
        printf '%s\n' "Legacy OverCrow artifacts: $diag_legacy_count exact artifact(s) detected"
    fi
}

report_service_state() {
    if [ "$diag_timeout_available" != true ]; then
        printf '%s\n' "overcrow-core.service: skipped: timeout unavailable"
        return
    fi
    if ! command -v systemctl >/dev/null 2>&1; then
        printf '%s\n' "overcrow-core.service: unavailable (systemctl not found)"
        return
    fi

    run_probe systemctl --user is-active overcrow-core.service
    if [ "$diag_probe_status" = timed_out ]; then
        printf '%s\n' "overcrow-core.service: timed out"
        return
    fi
    case "$diag_probe_output" in
        active|inactive|failed|activating|deactivating|reloading|maintenance)
            printf '%s\n' "overcrow-core.service: $diag_probe_output"
            ;;
        *)
            printf '%s\n' "overcrow-core.service: unavailable (user manager query failed)"
            ;;
    esac
}

report_hyprland_service() {
    if [ "$diag_timeout_available" != true ]; then
        printf '%s\n' "Hyprland bridge service: skipped: timeout unavailable"
        return
    fi
    if ! command -v systemctl >/dev/null 2>&1; then
        printf '%s\n' "Hyprland bridge service: unavailable (systemctl not found)"
        return
    fi

    run_probe systemctl --user is-active overcrow-hyprland.service
    if [ "$diag_probe_status" = timed_out ]; then
        printf '%s\n' "Hyprland bridge service: timed out"
        return
    fi
    case "$diag_probe_output" in
        active|inactive|failed|activating|deactivating|reloading|maintenance)
            printf '%s\n' "Hyprland bridge service: $diag_probe_output"
            ;;
        *)
            printf '%s\n' "Hyprland bridge service: unavailable (user manager query failed)"
            ;;
    esac
}

hyprland_session_active() {
    [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] && return 0
    case ${XDG_CURRENT_DESKTOP:-}:${DESKTOP_SESSION:-} in
        *Hyprland*|*hyprland*) return 0 ;;
    esac
    return 1
}

hyprland_signature_safe() {
    case $1 in
        ''|.|..|*[!A-Za-z0-9_.-]*) return 1 ;;
    esac
    return 0
}

captured_file_has_exact_line() {
    diag_exact_capture=$1
    diag_exact_wanted=$2
    [ -n "$diag_exact_capture" ] || return 1
    run_probe /usr/bin/gawk \
        -v wanted="$diag_exact_wanted" \
        '$0 == wanted { found = 1; exit } END { exit(found ? 0 : 1) }' \
        "$diag_exact_capture"
    [ "$diag_probe_status" = ok ] && [ "$diag_probe_exit" = 0 ]
}

report_hyprland_config() {
    if [ -n "${XDG_CONFIG_HOME:-}" ]; then
        diag_hypr_config_home=$XDG_CONFIG_HOME
    elif [ -n "${HOME:-}" ]; then
        diag_hypr_config_home=$HOME/.config
    else
        printf '%s\n' 'Hyprland OverCrow config: unavailable (config home unknown)'
        return
    fi

    diag_hypr_dir="$diag_hypr_config_home/hypr"
    diag_hypr_linked=false
    if [ -e "$diag_hypr_dir/hyprland.lua" ] || [ -L "$diag_hypr_dir/hyprland.lua" ]; then
        if diagnostic_control_file_capture "$diag_hypr_dir/hyprland.lua" 262144; then
            diag_hypr_main_capture=$diag_control_capture_file
        else
            diag_hypr_main_capture=
        fi
        if diagnostic_control_file_capture "$diag_hypr_dir/overcrow.lua" 65536; then
            diag_hypr_fragment_capture=$diag_control_capture_file
        else
            diag_hypr_fragment_capture=
        fi
        if captured_file_has_exact_line "$diag_hypr_main_capture" '-- BEGIN OVERCROW MANAGED' && \
            captured_file_has_exact_line "$diag_hypr_main_capture" 'require("overcrow")' && \
            captured_file_has_exact_line "$diag_hypr_fragment_capture" \
                '-- Managed by OverCrow. Changes are replaced by the installer.'; then
            diag_hypr_linked=true
        fi
        remove_diagnostic_capture "$diag_hypr_main_capture"
        remove_diagnostic_capture "$diag_hypr_fragment_capture"
    elif [ -e "$diag_hypr_dir/hyprland.conf" ] || [ -L "$diag_hypr_dir/hyprland.conf" ]; then
        if diagnostic_control_file_capture "$diag_hypr_dir/hyprland.conf" 262144; then
            diag_hypr_main_capture=$diag_control_capture_file
        else
            diag_hypr_main_capture=
        fi
        if diagnostic_control_file_capture "$diag_hypr_dir/overcrow.conf" 65536; then
            diag_hypr_fragment_capture=$diag_control_capture_file
        else
            diag_hypr_fragment_capture=
        fi
        if captured_file_has_exact_line "$diag_hypr_main_capture" '# BEGIN OVERCROW MANAGED' && \
            captured_file_has_exact_line "$diag_hypr_main_capture" \
                "source = $diag_hypr_dir/overcrow.conf" && \
            captured_file_has_exact_line "$diag_hypr_fragment_capture" \
                '# Managed by OverCrow. Changes are replaced by the installer.'; then
            diag_hypr_linked=true
        fi
        remove_diagnostic_capture "$diag_hypr_main_capture"
        remove_diagnostic_capture "$diag_hypr_fragment_capture"
    else
        printf '%s\n' 'Hyprland OverCrow config: not configured'
        return
    fi

    if [ "$diag_hypr_linked" = true ]; then
        printf '%s\n' 'Hyprland OverCrow config: linked'
    else
        printf '%s\n' 'Hyprland OverCrow config: incomplete or unlinked'
    fi
}

report_hyprland_sockets() {
    diag_hypr_runtime=${XDG_RUNTIME_DIR:-}
    diag_hypr_signature=${HYPRLAND_INSTANCE_SIGNATURE:-}
    case $diag_hypr_runtime in
        /*) ;;
        *)
            printf '%s\n' 'Hyprland command socket: unavailable (invalid runtime path)'
            printf '%s\n' 'Hyprland event socket: unavailable (invalid runtime path)'
            return
            ;;
    esac
    if ! hyprland_signature_safe "$diag_hypr_signature"; then
        printf '%s\n' 'Hyprland command socket: unavailable (invalid instance signature)'
        printf '%s\n' 'Hyprland event socket: unavailable (invalid instance signature)'
        return
    fi

    diag_hypr_instance="$diag_hypr_runtime/hypr/$diag_hypr_signature"
    if [ -S "$diag_hypr_instance/.socket.sock" ]; then
        printf '%s\n' 'Hyprland command socket: present'
    else
        printf '%s\n' 'Hyprland command socket: absent'
    fi
    if [ -S "$diag_hypr_instance/.socket2.sock" ]; then
        printf '%s\n' 'Hyprland event socket: present'
    else
        printf '%s\n' 'Hyprland event socket: absent'
    fi
}

report_hyprland_version() {
    run_probe hyprctl version
    case "$diag_probe_status" in
        timeout_unavailable)
            printf '%s\n' 'Hyprland version: skipped: timeout unavailable'
            ;;
        not_found)
            printf '%s\n' 'Hyprland version: unavailable (hyprctl not found)'
            ;;
        timed_out)
            printf '%s\n' 'Hyprland version: timed out'
            ;;
        ok)
            diag_hypr_first_line=${diag_probe_output%%'
'*}
            case $diag_hypr_first_line in
                'Hyprland '*)
                    diag_hypr_version=${diag_hypr_first_line#Hyprland }
                    diag_hypr_version=${diag_hypr_version%% *}
                    ;;
                *) diag_hypr_version= ;;
            esac
            case $diag_hypr_version in
                ''|*[!A-Za-z0-9._-]*)
                    printf '%s\n' 'Hyprland version: unavailable (unrecognized response)'
                    ;;
                *)
                    printf '%s\n' "Hyprland version: $diag_hypr_version"
                    ;;
            esac
            ;;
        *)
            printf '%s\n' 'Hyprland version: unavailable (hyprctl query failed)'
            ;;
    esac
}

report_hyprland_config_errors() {
    run_probe hyprctl configerrors
    case "$diag_probe_status" in
        timeout_unavailable)
            printf '%s\n' 'Hyprland config errors: skipped: timeout unavailable'
            ;;
        not_found)
            printf '%s\n' 'Hyprland config errors: unavailable (hyprctl not found)'
            ;;
        timed_out)
            printf '%s\n' 'Hyprland config errors: timed out'
            ;;
        ok)
            if [ -z "$diag_probe_output" ] || [ "$diag_probe_output" = 'no errors' ]; then
                printf '%s\n' 'Hyprland config errors: none'
            else
                printf '%s\n' 'Hyprland config errors: reported (details withheld)'
            fi
            ;;
        *)
            printf '%s\n' 'Hyprland config errors: unavailable (hyprctl query failed)'
            ;;
    esac
}

report_hyprland_overlays() {
    run_probe hyprctl -j clients
    case "$diag_probe_status" in
        timeout_unavailable)
            printf '%s\n' 'Hyprland overlay windows: skipped: timeout unavailable'
            return
            ;;
        not_found)
            printf '%s\n' 'Hyprland overlay windows: unavailable (hyprctl not found)'
            return
            ;;
        timed_out)
            printf '%s\n' 'Hyprland overlay windows: timed out'
            return
            ;;
        oversized)
            printf '%s\n' 'Hyprland overlay windows: unavailable (response too large)'
            return
            ;;
        ok) ;;
        *)
            printf '%s\n' 'Hyprland overlay windows: unavailable (hyprctl query failed)'
            return
            ;;
    esac

    run_probe_with_input "$diag_probe_output" jq -er \
        'if type == "array" then [ .[] | select(type == "object" and .class == "io.github.overcrow.Overlay") ] | length else error("clients is not an array") end'
    case "$diag_probe_status" in
        timeout_unavailable)
            printf '%s\n' 'Hyprland overlay windows: skipped: timeout unavailable'
            ;;
        not_found)
            printf '%s\n' 'Hyprland overlay windows: unavailable (jq not found)'
            ;;
        timed_out)
            printf '%s\n' 'Hyprland overlay windows: JSON parser timed out'
            ;;
        oversized)
            printf '%s\n' 'Hyprland overlay windows: unavailable (parser response too large)'
            ;;
        ok)
            case $diag_probe_output in
                ''|*[!0-9]*)
                    printf '%s\n' 'Hyprland overlay windows: malformed response'
                    ;;
                *)
                    printf '%s\n' "Hyprland overlay windows: $diag_probe_output"
                    ;;
            esac
            ;;
        *)
            printf '%s\n' 'Hyprland overlay windows: malformed response'
            ;;
    esac
}

report_hyprland_runtime_shortcut() {
    run_probe hyprctl -j binds
    case "$diag_probe_status" in
        timeout_unavailable)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: skipped: timeout unavailable'
            return
            ;;
        not_found)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: unavailable (hyprctl not found)'
            return
            ;;
        timed_out)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: timed out'
            return
            ;;
        oversized)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: unavailable (response too large)'
            return
            ;;
        ok) ;;
        *)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: unavailable (hyprctl query failed)'
            return
            ;;
    esac

    run_probe_with_input "$diag_probe_output" jq -er \
        'if type != "array" then error("bindings is not an array") else [ .[] | select(type == "object" and .modmask == 72 and .key == "O") ] as $matching | if ($matching | length) == 0 then "inactive" elif ($matching | length) == 1 and $matching[0].description == "OverCrow overlay" and $matching[0].dispatcher == "global" and ($matching[0].arg == "com.playervox.OverCrow:toggle-overlay" or $matching[0].arg == ":toggle-overlay") then "active" else "conflict" end end'
    case "$diag_probe_status:$diag_probe_output" in
        ok:active)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: active (SUPER + ALT + O)'
            ;;
        ok:inactive)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: inactive'
            ;;
        ok:conflict)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: conflict on SUPER + ALT + O'
            ;;
        timed_out:*)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: JSON parser timed out'
            ;;
        oversized:*)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: unavailable (parser response too large)'
            ;;
        *)
            printf '%s\n' 'Hyprland OverCrow runtime shortcut: malformed response'
            ;;
    esac
}

report_hyprland() {
    if ! hyprland_session_active; then
        printf '%s\n' 'Hyprland: skipped (not a Hyprland session)'
        return
    fi
    report_hyprland_version
    report_hyprland_sockets
    report_hyprland_service
    report_hyprland_config
    report_hyprland_config_errors
    report_hyprland_runtime_shortcut
    report_hyprland_overlays
}

diag_current_uid=
diag_current_uid_status=unchecked

resolve_current_uid() {
    if [ "$diag_current_uid_status" != unchecked ]; then
        return
    fi

    run_probe id -u
    case "$diag_probe_status" in
        timeout_unavailable)
            diag_current_uid_status=timeout_unavailable
            ;;
        not_found)
            diag_current_uid_status=not_found
            ;;
        timed_out)
            diag_current_uid_status=timed_out
            ;;
        ok)
            case "$diag_probe_output" in
                ''|*[!0-9]*)
                    diag_current_uid_status=invalid
                    ;;
                *)
                    diag_current_uid=$diag_probe_output
                    diag_current_uid_status=ok
                    ;;
            esac
            ;;
        *)
            diag_current_uid_status=error
            ;;
    esac
}

report_process() {
    diag_process_name=$1
    resolve_current_uid
    case "$diag_current_uid_status" in
        timeout_unavailable)
            printf '%s\n' "$diag_process_name process: skipped: timeout unavailable"
            return
            ;;
        not_found)
            printf '%s\n' "$diag_process_name process: skipped: current UID unavailable (id not found)"
            return
            ;;
        timed_out)
            printf '%s\n' "$diag_process_name process: skipped: current UID probe timed out"
            return
            ;;
        invalid|error)
            printf '%s\n' "$diag_process_name process: skipped: current UID unavailable (id -u failed)"
            return
            ;;
    esac
    if ! command -v pgrep >/dev/null 2>&1; then
        printf '%s\n' "$diag_process_name process: unavailable (pgrep not found)"
        return
    fi

    diag_process_pattern="^([^[:space:]]*/)?$diag_process_name([[:space:]]|$)"
    run_probe pgrep -u "$diag_current_uid" -f "$diag_process_pattern"
    case "$diag_probe_status" in
        timed_out)
            printf '%s\n' "$diag_process_name process: timed out"
            return
            ;;
    esac
    case "$diag_probe_exit" in
        0)
            diag_pid_list=
            for diag_pid in $diag_probe_output; do
                case "$diag_pid" in
                    ''|*[!0-9]*)
                        ;;
                    *)
                        diag_pid_list="${diag_pid_list}${diag_pid_list:+ }$diag_pid"
                        ;;
                esac
            done
            if [ -n "$diag_pid_list" ]; then
                printf '%s\n' "$diag_process_name process: running (PIDs: $diag_pid_list)"
            else
                printf '%s\n' "$diag_process_name process: unavailable (invalid pgrep output)"
            fi
            ;;
        1)
            printf '%s\n' "$diag_process_name process: not running"
            ;;
        *)
            printf '%s\n' "$diag_process_name process: unavailable (pgrep failed)"
            ;;
    esac
}

read_kwinrc_value() {
    diag_kwinrc_capture=$1
    run_probe /usr/bin/gawk \
        -v key="${kwin_id}Enabled=" \
        '$0 == "[Plugins]" { in_plugins = 1; next } /^\[.*\]$/ { in_plugins = 0; next } in_plugins && index($0, key) == 1 { print substr($0, length(key) + 1); exit }' \
        "$diag_kwinrc_capture"
    [ "$diag_probe_status" = ok ] || return 1
    printf '%s\n' "$diag_probe_output"
}

print_kwin_enabled() {
    diag_enabled_value=$1
    diag_enabled_origin=$2
    case "$diag_enabled_value" in
        true|True|TRUE|1|yes|Yes|YES|on|On|ON)
            printf '%s\n' "KWin package enabled: yes$diag_enabled_origin"
            ;;
        false|False|FALSE|0|no|No|NO|off|Off|OFF)
            printf '%s\n' "KWin package enabled: no$diag_enabled_origin"
            ;;
        '')
            printf '%s\n' "KWin package enabled: not configured$diag_enabled_origin"
            ;;
        *)
            printf '%s\n' "KWin package enabled: unavailable (unrecognized value$diag_enabled_origin)"
            ;;
    esac
}

report_kwin_package() {
    if [ -n "${XDG_DATA_HOME:-}" ]; then
        diag_data_home=$XDG_DATA_HOME
    elif [ -n "${HOME:-}" ]; then
        diag_data_home=$HOME/.local/share
    else
        diag_data_home=
    fi

    if [ -n "${XDG_CONFIG_HOME:-}" ]; then
        diag_config_home=$XDG_CONFIG_HOME
    elif [ -n "${HOME:-}" ]; then
        diag_config_home=$HOME/.config
    else
        diag_config_home=
    fi

    diag_package_metadata=
    if [ -n "$diag_data_home" ]; then
        diag_package_metadata="$diag_data_home/kwin/scripts/$kwin_id/metadata.json"
    fi

    diag_package_tool=false
    diag_package_installed=false
    run_probe kpackagetool6 --type KWin/Script --show "$kwin_id"
    diag_package_probe_status=$diag_probe_status
    diag_package_probe_exit=$diag_probe_exit
    if [ "$diag_package_probe_status" != not_found ] && \
        [ "$diag_package_probe_status" != timeout_unavailable ]; then
        diag_package_tool=true
        if [ "$diag_package_probe_status" = ok ]; then
            diag_package_installed=true
            printf '%s\n' "KWin package $kwin_id: installed"
        fi
    fi

    if [ "$diag_package_installed" = false ]; then
        if [ "$diag_package_probe_status" = timeout_unavailable ]; then
            printf '%s\n' "KWin package $kwin_id: skipped: timeout unavailable"
        elif [ -n "$diag_package_metadata" ] && \
            diagnostic_control_file_capture "$diag_package_metadata" 262144; then
            printf '%s\n' "KWin package $kwin_id: installed (metadata file)"
            /usr/bin/rm -f -- "$diag_control_capture_file"
        elif [ "$diag_package_probe_status" = timed_out ]; then
            printf '%s\n' "KWin package $kwin_id: timed out"
        elif [ "$diag_package_probe_status" = error ] && \
            [ "$diag_package_probe_exit" -ne 1 ]; then
            printf '%s\n' "KWin package $kwin_id: unavailable (kpackagetool6 query failed)"
        elif [ "$diag_package_tool" = true ]; then
            printf '%s\n' "KWin package $kwin_id: not installed"
        else
            printf '%s\n' "KWin package $kwin_id: unavailable (tool and readable metadata absent)"
        fi
    fi

    diag_kwinrc=
    if [ -n "$diag_config_home" ]; then
        diag_kwinrc="$diag_config_home/kwinrc"
    fi

    diag_enabled_value=
    diag_enabled_origin=
    diag_enabled_read=false
    run_probe kreadconfig6 \
        --file kwinrc \
        --group Plugins \
        --key "${kwin_id}Enabled"
    diag_enabled_probe_status=$diag_probe_status
    if [ "$diag_enabled_probe_status" = ok ]; then
        diag_enabled_value=$diag_probe_output
        diag_enabled_read=true
    fi

    if [ "$diag_enabled_probe_status" != timeout_unavailable ] && \
        [ "$diag_enabled_read" = false ] && [ -n "$diag_kwinrc" ]; then
        if diagnostic_control_file_capture "$diag_kwinrc" 262144; then
            diag_kwinrc_capture=$diag_control_capture_file
            if diag_enabled_value=$(read_kwinrc_value "$diag_kwinrc_capture"); then
                diag_enabled_origin=" (kwinrc)"
                diag_enabled_read=true
            fi
            /usr/bin/rm -f -- "$diag_kwinrc_capture"
        fi
    fi

    if [ "$diag_enabled_read" = true ]; then
        print_kwin_enabled "$diag_enabled_value" "$diag_enabled_origin"
    elif [ "$diag_enabled_probe_status" = timeout_unavailable ]; then
        printf '%s\n' "KWin package enabled: skipped: timeout unavailable"
    elif [ "$diag_enabled_probe_status" = timed_out ]; then
        printf '%s\n' "KWin package enabled: timed out"
    elif [ "$diag_enabled_probe_status" = error ]; then
        printf '%s\n' "KWin package enabled: unavailable (kreadconfig6 query failed)"
    else
        printf '%s\n' "KWin package enabled: unavailable (tool and readable kwinrc absent)"
    fi
}

filter_wayland_globals() {
    printf '%s\n' "$1" |
        while IFS= read -r diag_wayland_line || [ -n "$diag_wayland_line" ]; do
            case "$diag_wayland_line" in
                *"interface: 'wl_compositor'"*|\
                *"interface: 'xdg_wm_base'"*|\
                *"interface: 'org_kde_plasma_shell'"*|\
                *"interface: 'org_kde_kwin_server_decoration_manager'"*|\
                *"interface: 'zwlr_layer_shell_v1'"*|\
                *"interface: 'ext_layer_shell_v1'"*|\
                *"interface: 'zxdg_output_manager_v1'"*)
                    printf '%s\n' "$diag_wayland_line"
                    ;;
            esac
        done
}

report_wayland_globals() {
    if [ "${XDG_SESSION_TYPE:-}" != wayland ]; then
        printf '%s\n' "Wayland globals: skipped (not a Wayland session)"
        return
    fi
    if [ "$diag_timeout_available" != true ]; then
        printf '%s\n' "Wayland globals: skipped: timeout unavailable"
        return
    fi

    run_probe wayland-info
    case "$diag_probe_status" in
        ok)
            ;;
        not_found)
            printf '%s\n' "Wayland globals: unavailable (wayland-info not found)"
            return
            ;;
        timed_out)
            printf '%s\n' "Wayland globals: timed out"
            return
            ;;
        *)
            printf '%s\n' "Wayland globals: unavailable (wayland-info query failed)"
            return
            ;;
    esac

    diag_wayland_relevant=$(filter_wayland_globals "$diag_probe_output")
    if [ -n "$diag_wayland_relevant" ]; then
        printf '%s\n' "Wayland globals (relevant):"
        printf '%s\n' "$diag_wayland_relevant"
    else
        printf '%s\n' "Wayland globals: none of the relevant interfaces were reported"
    fi
}

report_x11_atom() {
    diag_xprop_atom=$1
    run_probe xprop -root "$diag_xprop_atom"
    case "$diag_probe_status" in
        timed_out)
            printf '%s\n' "X11 EWMH $diag_xprop_atom: timed out"
            return
            ;;
        ok)
            ;;
        *)
            printf '%s\n' "X11 EWMH $diag_xprop_atom: unavailable (xprop query failed)"
            return
            ;;
    esac

    case "$diag_probe_output" in
        *"$diag_xprop_atom:"*"no such atom"*|*"$diag_xprop_atom"*"not found"*)
            printf '%s\n' "X11 EWMH $diag_xprop_atom: absent"
            ;;
        *"$diag_xprop_atom("*|*"$diag_xprop_atom ="*|*"$diag_xprop_atom:"*)
            printf '%s\n' "X11 EWMH $diag_xprop_atom: present"
            ;;
        *)
            printf '%s\n' "X11 EWMH $diag_xprop_atom: unavailable (unrecognized xprop response)"
            ;;
    esac
}

report_x11_ewmh() {
    if [ -z "${DISPLAY:-}" ]; then
        printf '%s\n' "X11 EWMH: skipped (DISPLAY unavailable)"
        return
    fi
    if [ "$diag_timeout_available" != true ]; then
        printf '%s\n' "X11 EWMH: skipped: timeout unavailable"
        return
    fi
    if ! command -v xprop >/dev/null 2>&1; then
        printf '%s\n' "X11 EWMH: unavailable (xprop not found)"
        return
    fi

    report_x11_atom _NET_SUPPORTING_WM_CHECK
    report_x11_atom _NET_ACTIVE_WINDOW
}

printf '%s\n' "OverCrow diagnostic report (read-only)"
print_environment_value XDG_SESSION_TYPE "${XDG_SESSION_TYPE:-}"
print_environment_value XDG_CURRENT_DESKTOP "${XDG_CURRENT_DESKTOP:-}"
print_environment_value DESKTOP_SESSION "${DESKTOP_SESSION:-}"
if [ -n "${DISPLAY:-}" ]; then
    printf '%s\n' "DISPLAY: available"
else
    printf '%s\n' "DISPLAY: unavailable"
fi
printf '\n'

report_dbus_owner
report_lifecycle_settings
report_service_state
report_shortcut_availability
report_process overcrow-core
report_process overcrow-overlay
report_legacy_artifacts
printf '\n'

report_hyprland
printf '\n'

report_kwin_package
printf '\n'

report_wayland_globals
report_x11_ewmh
