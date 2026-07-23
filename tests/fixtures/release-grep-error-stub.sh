#!/bin/sh
set -eu

recursive=0
target=0
for argument do
    if test "$argument" = '-R'; then
        recursive=1
    fi
    case ${OVERCROW_GREP_ERROR_MODE:-}:$argument in
        token:'@OVERCROW_BINDIR@|%h/\.local/bin') target=1 ;;
        private:*'github\.com(:[0-9]+)'*) target=1 ;;
    esac
done

if test "$recursive" -eq 1 && test "$target" -eq 1; then
    exit 2
fi
exec /usr/bin/grep "$@"
