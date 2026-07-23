#!/bin/sh
set -eu

: "${OVERCROW_DANGLING_HOLD:?}"

# stage.sh invokes: mv -T -n -- WORKING DESTINATION.
working=${4:?}
/usr/bin/mv -T -- "$working" "$OVERCROW_DANGLING_HOLD"
/usr/bin/ln -s "$OVERCROW_DANGLING_HOLD/missing" "$working"
exit 1
