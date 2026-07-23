#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
helper="$root/scripts/integrate-user.sh"
library="$root/scripts/lib/hyprland-config.sh"

fail() {
    printf '%s\n' "integration smoke test failed: $1" >&2
    exit 1
}

sh -n "$helper" "$library"

grep -Fq "installed_helper='/usr/lib/overcrow/overcrow-integrate'" "$helper" ||
    fail 'the installed helper path is not fixed'
grep -Fq "installed_library='/usr/lib/overcrow/hyprland-config.sh'" "$helper" ||
    fail 'the installed library path is not fixed'
grep -Fq "installed_share='/usr/share/overcrow/integrations'" "$helper" ||
    fail 'the installed integration path is not fixed'
grep -Fq "timeout_program='/usr/bin/timeout'" "$helper" ||
    fail 'the helper does not use the fixed timeout program'
# shellcheck disable=SC2016 # Verify literal variables in the helper source.
grep -Fq '"$timeout_program" --signal=TERM --kill-after=1s 2s "$@"' "$helper" ||
    fail 'the helper command bound is not explicit'
# shellcheck disable=SC2016 # Verify literal variables in the helper source.
grep -Fq '[ "$script_path" = "$installed_helper" ]' "$helper" ||
    fail 'the helper is not restricted to the installed layout'

kwin_metadata_sha256=$(/usr/bin/sha256sum \
    "$root/integrations/kwin/metadata.json")
kwin_metadata_sha256=${kwin_metadata_sha256%% *}
kwin_main_sha256=$(/usr/bin/sha256sum \
    "$root/integrations/kwin/contents/code/main.js")
kwin_main_sha256=${kwin_main_sha256%% *}
grep -Fq "kwin_current_metadata_sha256='$kwin_metadata_sha256'" "$helper" ||
    fail 'the pinned KWin metadata fingerprint is stale'
grep -Fq "kwin_current_main_sha256='$kwin_main_sha256'" "$helper" ||
    fail 'the pinned KWin script fingerprint is stale'

grep -Fq "overcrow_hypr_timeout_program='/usr/bin/timeout'" "$library" ||
    fail 'the Hyprland library does not use the fixed timeout program'
# shellcheck disable=SC2016 # Verify literal variables in the library source.
grep -Fq '"$overcrow_hypr_timeout_program" --signal=TERM --kill-after=1s 2s "$@"' \
    "$library" || fail 'the Hyprland command bound is not explicit'

if rg -n 'overcrow-supervise|supervisor_program|source_parent|source_helper' \
        "$helper" "$library" > /dev/null; then
    fail 'a compiled-supervisor or source-layout path remains'
fi
