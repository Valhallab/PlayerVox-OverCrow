#!/bin/sh
set -eu

: "${OVERCROW_RACE_DESTINATION:?}"
/usr/bin/install -d -m 0755 "$OVERCROW_RACE_DESTINATION"
printf '%s\n' 'racing owner' > "$OVERCROW_RACE_DESTINATION/race-owner"
exec /usr/bin/mv "$@"
