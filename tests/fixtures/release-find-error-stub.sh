#!/bin/sh
set -eu

if test "${OVERCROW_FIND_ERROR_MODE:-}" = source; then
    for argument do
        if test "$argument" = 'Cargo.toml'; then
            exit 2
        fi
    done
fi
exec /usr/bin/find "$@"
