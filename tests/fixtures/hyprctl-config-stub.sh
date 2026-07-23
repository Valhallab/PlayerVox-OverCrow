#!/bin/sh
set -eu

: "${HYPR_TEST_STATE:?HYPR_TEST_STATE must be set}"

if [ -n "${HYPR_TEST_COMMAND_LOG:-}" ]; then
    printf '%s\n' "${1:-}" >> "$HYPR_TEST_COMMAND_LOG"
fi

if [ "${HYPR_TEST_STATUS_125:-}" = "${1:-}" ]; then
    /usr/bin/setsid /usr/bin/sh -c \
        '/usr/bin/sleep 1; /usr/bin/touch "$1"' sh \
        "$HYPR_TEST_STATUS_125_SENTINEL" &
    printf '%s\n' "$!" > "$HYPR_TEST_STATUS_125_PID"
    exit 125
fi

if [ "${HYPR_TEST_HANG:-}" = "${1:-}" ]; then
    printf '%s\n' "$$" > "$HYPR_TEST_STATE/hanging.pid"
    trap 'exit 143' TERM INT
    while :; do :; done
fi

case ${1:-} in
    configerrors)
        if [ -f "$HYPR_TEST_STATE/reloaded" ] && \
            [ "${HYPR_TEST_FAIL_AFTER_RELOAD:-0}" = 1 ]; then
            printf '%s\n' 'Config error in file overcrow.conf'
        else
            printf '%s\n' 'no errors'
        fi
        ;;
    reload)
        mkdir -p "$HYPR_TEST_STATE"
        touch "$HYPR_TEST_STATE/reloaded"
        printf '%s\n' 'ok'
        ;;
    *)
        printf 'unexpected hyprctl invocation: %s\n' "$*" >&2
        exit 64
        ;;
esac
