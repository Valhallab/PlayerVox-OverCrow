#!/bin/sh
set -eu

name=${0##*/}
printf '%s %s\n' "$name" "$*" >> "${OVERCROW_DIAG_STUB_LOG:?}"

case ",${OVERCROW_DIAG_FAIL_TARGETS:-}," in
    *,"$name",*)
        exit 69
        ;;
esac

case ",${OVERCROW_DIAG_IGNORE_TERM_TARGETS:-}," in
    *,"$name",*)
        trap '' TERM
        while :; do
            :
        done
        ;;
esac

case "$name" in
    id)
        [ "$*" = "-u" ]
        printf '%s\n' "${OVERCROW_DIAG_STUB_UID:-1000}"
        ;;
    busctl)
        case "$*" in
            "--user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus NameHasOwner s io.github.overcrow.Core1")
                printf '%s\n' "b true"
                ;;
            "--user call io.github.overcrow.Core1 /io/github/overcrow/Core1 io.github.overcrow.Core1 ShortcutAvailability")
                printf '%s\n' 's "available"'
                ;;
            *) exit 64 ;;
        esac
        ;;
    gdbus)
        [ "$*" = "call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner io.github.overcrow.Core1" ]
        printf '%s\n' "(true,)"
        ;;
    systemctl)
        case "$*" in
            "--user is-active overcrow-core.service"|\
            "--user is-active overcrow-hyprland.service") ;;
            *) exit 64 ;;
        esac
        printf '%s\n' "active"
        ;;
    hyprctl)
        case "$*" in
            version)
                printf '%s\n' 'Hyprland 0.55.2 built from branch main'
                ;;
            "-j clients")
                if [ "${OVERCROW_DIAG_OVERSIZED_HYPRLAND:-0}" = 1 ]; then
                    /usr/bin/head -c 8388608 /dev/zero | /usr/bin/tr '\000' x
                    : > "${OVERCROW_DIAG_OVERSIZED_COMPLETED_MARKER:?}"
                elif [ "${OVERCROW_DIAG_MALFORMED_HYPRLAND:-0}" = 1 ]; then
                    printf '%s\n' '[not-json]'
                elif [ "${OVERCROW_DIAG_MINIFIED_HYPRLAND:-0}" = 1 ]; then
                    printf '%s\n' '[{"class":"io.github.overcrow.Overlay"},{"class":"unrelated"}]'
                else
                    printf '%s\n' \
                        '[' \
                        '  {"class": "io.github.overcrow.Overlay", "title": "private game title"},' \
                        '  {"class": "io.github.overcrow.Overlay.fake", "title": "private browser title"}' \
                        ']'
                fi
                ;;
            "-j binds")
                case ${OVERCROW_DIAG_SHORTCUT_STATE:-active} in
                    active)
                        printf '%s\n' \
                            '[{"modmask":72,"key":"O","description":"OverCrow overlay","dispatcher":"global","arg":"com.playervox.OverCrow:toggle-overlay"}]'
                        ;;
                    legacy)
                        printf '%s\n' \
                            '[{"modmask":72,"key":"O","description":"OverCrow overlay","dispatcher":"global","arg":":toggle-overlay"}]'
                        ;;
                    inactive)
                        printf '%s\n' '[]'
                        ;;
                    conflict)
                        printf '%s\n' \
                            '[{"modmask":72,"key":"O","description":"Foreign action","dispatcher":"exec","arg":"foreign"}]'
                        ;;
                    *) exit 64 ;;
                esac
                ;;
            configerrors)
                if [ "${OVERCROW_DIAG_HYPRLAND_ERRORS:-0}" = 1 ]; then
                    printf '%s\n' 'Config error in a private user path'
                else
                    :
                fi
                ;;
            *) exit 64 ;;
        esac
        ;;
    pgrep)
        case "$*" in
            "-u 1000 -f ^([^[:space:]]*/)?overcrow-core([[:space:]]|$)")
                printf '%s\n' "4242" "4243"
                ;;
            "-u 1000 -f ^([^[:space:]]*/)?overcrow-overlay([[:space:]]|$)")
                printf '%s\n' "5252"
                ;;
            *)
                exit 64
                ;;
        esac
        ;;
    kpackagetool6)
        [ "$*" = "--type KWin/Script --show io.github.overcrow.kwin" ]
        printf '%s\n' "Id: io.github.overcrow.kwin"
        ;;
    kreadconfig6)
        [ "$*" = "--file kwinrc --group Plugins --key io.github.overcrow.kwinEnabled" ]
        printf '%s\n' "true"
        ;;
    timeout)
        [ "$1" = "--signal=TERM" ]
        [ "$2" = "--kill-after=1s" ]
        [ "$3" = "3s" ]
        shift 3
        swap_last=
        for swap_arg in "$@"; do
            swap_last=$swap_arg
        done
        if [ "$1" = /usr/bin/cmp ] && \
            [ -n "${OVERCROW_DIAG_SWAP_PATH:-}" ] && \
            [ -n "${OVERCROW_DIAG_SWAP_REPLACEMENT:-}" ]; then
            case $swap_last in
                */overcrow-legacy-lua.*)
                    /usr/bin/mv -f -- "$OVERCROW_DIAG_SWAP_REPLACEMENT" \
                        "$OVERCROW_DIAG_SWAP_PATH"
                    ;;
            esac
        fi
        if [ -n "${OVERCROW_DIAG_REAL_TIMEOUT:-}" ]; then
            exec "$OVERCROW_DIAG_REAL_TIMEOUT" \
                --signal=TERM --kill-after=1s 3s "$@"
        fi
        case ",${OVERCROW_DIAG_HANG_TARGETS:-${OVERCROW_DIAG_HANG_TARGET:-}}," in
            *,"$1",*)
                exit 124
                ;;
        esac
        case ",${OVERCROW_DIAG_KILL_TARGETS:-}," in
            *,"$1",*)
                exit 137
                ;;
        esac
        exec "$@"
        ;;
    wayland-info)
        [ "$#" -eq 0 ]
        printf '%s\n' \
            "interface: 'wl_compositor', version: 6, name: 1" \
            "interface: 'xdg_wm_base', version: 6, name: 9" \
            "interface: 'org_kde_plasma_shell', version: 8, name: 14" \
            "interface: 'irrelevant_test_global', version: 1, name: 99"
        ;;
    xprop)
        case "$*" in
            "-root _NET_SUPPORTING_WM_CHECK")
                printf '%s\n' "_NET_SUPPORTING_WM_CHECK(WINDOW): window id # 0x100001"
                ;;
            "-root _NET_ACTIVE_WINDOW")
                printf '%s\n' "_NET_ACTIVE_WINDOW:  no such atom on any window."
                ;;
            *)
                exit 64
                ;;
        esac
        ;;
    install|cp|mv|rm|mkdir|touch|chmod|chown|kwriteconfig6|qdbus6|qdbus)
        printf '%s\n' "mutating or out-of-contract command invoked: $name $*" >&2
        exit 65
        ;;
    *)
        printf '%s\n' "unexpected stub command invoked: $name $*" >&2
        exit 66
        ;;
esac
