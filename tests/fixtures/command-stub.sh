#!/bin/sh
set -eu

name=${0##*/}
printf '%s %s\n' "$name" "$*" >> "$OVERCROW_STUB_LOG"

interrupt_fragment_target() {
    interrupt_target=${OVERCROW_STUB_TARGET_PID:?}
    case $interrupt_target in ''|*[!0-9]*) exit 64 ;; esac
    [ "$interrupt_target" -gt 1 ] || exit 64
    /usr/bin/kill "-${OVERCROW_STUB_FRAGMENT_SIGNAL:-TERM}" \
        "$interrupt_target"
}

if [ "$name" = "id" ]; then
    printf '%s\n' "${OVERCROW_STUB_UID:-1000}"
    exit 0
fi

if [ "$name" = "cargo" ]; then
    crate_path=
    install_root=
    previous=
    for argument in "$@"; do
        case $previous in
            --path) crate_path=$argument ;;
            --root) install_root=$argument ;;
        esac
        previous=$argument
    done
    [ -n "$crate_path" ] && [ -n "$install_root" ] || exit 64
    /usr/bin/mkdir -p -- "$install_root/bin"
    binary_name=${crate_path##*/}
    /usr/bin/printf '%s\n' '#!/bin/sh' 'exit 0' > "$install_root/bin/$binary_name"
    /usr/bin/chmod 755 "$install_root/bin/$binary_name"
    if [ "${OVERCROW_STUB_INTERRUPT_CARGO_AFTER:-}" = "$binary_name" ]; then
        /usr/bin/kill -TERM "$PPID"
        /usr/bin/sleep 1
        exit 143
    fi
    exit 0
fi

if [ "$name" = cmp ]; then
    if [ "${OVERCROW_STUB_INTERRUPT_FRAGMENT_COMPARE:-0}" = 1 ]; then
        interrupt_fragment_target
    fi
    exec /usr/bin/cmp "$@"
fi

if [ "$name" = systemctl ]; then
    last_argument=
    for argument in "$@"; do
        last_argument=$argument
    done
    case " $* " in
        *" --user show --property=FragmentPath --value "*)
            if [ "${OVERCROW_STUB_INTERRUPT_FRAGMENT_SHOW:-0}" = 1 ]; then
                interrupt_fragment_target
            fi
            if [ "${OVERCROW_STUB_FAIL_SYSTEMCTL_SHOW:-0}" = 1 ]; then
                exit 74
            fi
            if [ -n "${OVERCROW_STUB_FRAGMENT_DIR:-}" ] && \
                [ -f "$OVERCROW_STUB_FRAGMENT_DIR/$last_argument" ]; then
                /usr/bin/cat -- "$OVERCROW_STUB_FRAGMENT_DIR/$last_argument"
            elif [ -n "${OVERCROW_STUB_RELOAD_MARKER:-}" ] && \
                [ -e "$OVERCROW_STUB_RELOAD_MARKER" ] && \
                [ -n "${OVERCROW_STUB_FRAGMENT_AFTER_RELOAD:-}" ]; then
                printf '%s\n' "$OVERCROW_STUB_FRAGMENT_AFTER_RELOAD"
            elif [ -n "${OVERCROW_STUB_FRAGMENT_PATH:-}" ]; then
                printf '%s\n' "$OVERCROW_STUB_FRAGMENT_PATH"
            elif [ -e "${XDG_DATA_HOME:?}/systemd/user/$last_argument" ]; then
                printf '%s\n' "$XDG_DATA_HOME/systemd/user/$last_argument"
            fi
            exit 0
            ;;
        *" --user daemon-reload "*)
            if [ "${OVERCROW_STUB_FAIL_DAEMON_RELOAD:-0}" = 1 ]; then
                exit 75
            fi
            if [ -n "${OVERCROW_STUB_RELOAD_MARKER:-}" ]; then
                /usr/bin/touch "$OVERCROW_STUB_RELOAD_MARKER"
            fi
            ;;
        *" --user stop "*)
            if [ "${OVERCROW_STUB_FAIL_SYSTEMCTL_STOP:-0}" = 1 ]; then
                exit 73
            fi
            if [ -n "${OVERCROW_STUB_INTERRUPT_AFTER_STOP:-}" ] &&
                [ "$last_argument" = "$OVERCROW_STUB_INTERRUPT_AFTER_STOP" ]; then
                /usr/bin/kill -TERM "$PPID"
                /usr/bin/sleep 1
                exit 143
            fi
            ;;
    esac
fi

if [ "$name" = "kpackagetool6" ]; then
    case " $* " in
        *" --show "*)
            [ -f "$OVERCROW_KPACKAGE_STATE" ]
            ;;
        *" --install "*)
            : > "$OVERCROW_KPACKAGE_STATE"
            ;;
        *" --upgrade "*)
            [ -f "$OVERCROW_KPACKAGE_STATE" ]
            ;;
        *" --remove "*)
            rm -f -- "$OVERCROW_KPACKAGE_STATE"
            ;;
    esac
fi
