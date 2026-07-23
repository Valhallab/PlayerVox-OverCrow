#!/bin/sh
set -eu

: "${OVERCROW_SIGNAL_STAGE_PID:?}"
/usr/bin/install "$@"
kill -TERM "$OVERCROW_SIGNAL_STAGE_PID"
exit 143
