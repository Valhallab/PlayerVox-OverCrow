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
grep -Fq "qdbus_arch_program='/usr/bin/qdbus6'" "$helper" ||
    fail 'the helper does not retain the fixed Arch qdbus path'
grep -Fq "qdbus_fedora_program='/usr/bin/qdbus-qt6'" "$helper" ||
    fail 'the helper does not support the fixed Fedora qdbus path'
grep -Fq 'canonical_default_home' "$helper" ||
    fail 'the helper does not normalize the default immutable-desktop home'
grep -Fq '/var/home/' "$helper" ||
    fail 'the helper does not recognize the Fedora Atomic home layout'
# shellcheck disable=SC2016 # Verify literal variables in the helper source.
grep -Fq '/Scripting unloadScript "$kwin_id"' "$helper" ||
    fail 'the helper does not reload an upgraded KWin script'
awk '
    /\/Scripting unloadScript/ { unload = NR }
    /\/KWin reconfigure/ { reconfigure = NR }
    END { exit !(unload && reconfigure && unload < reconfigure) }
' "$helper" || fail 'the helper does not unload the old KWin script before reconfiguration'
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
grep -Fq \
    "kwin_legacy_pre_alpha_3_metadata_sha256='d1a3ad62abe425afde4fd04251fc45de8f4a9855e661f7271449aa339211ec6d'" \
    "$helper" || fail 'the pre-alpha 3 KWin metadata fingerprint was not retained'
grep -Fq \
    "kwin_legacy_pre_alpha_3_main_sha256='9fc7a92d1f2936e454ac83bc7b187110b7d22fae5f93bd355dd99557e656259d'" \
    "$helper" || fail 'the pre-alpha 3 KWin script fingerprint was not retained'
grep -Fq \
    "kwin_legacy_pre_alpha_2_metadata_sha256='72844f4e860c98974fa240a4fb1620d0ea25db6cd9facfe46dde3dbdb9adeb70'" \
    "$helper" || fail 'the pre-alpha 2 KWin metadata fingerprint was not retained'
grep -Fq \
    "kwin_legacy_pre_alpha_2_main_sha256='9fc7a92d1f2936e454ac83bc7b187110b7d22fae5f93bd355dd99557e656259d'" \
    "$helper" || fail 'the pre-alpha 2 KWin script fingerprint was not retained'
grep -Fq \
    "kwin_legacy_pre_alpha_1_metadata_sha256='d3f2a92714dbd0fb2c497341d9ae7eabd5498e7c87047a77dd7dcf9c54889f83'" \
    "$helper" || fail 'the pre-alpha 1 KWin metadata fingerprint was not retained'
grep -Fq \
    "kwin_legacy_pre_alpha_1_main_sha256='9fc7a92d1f2936e454ac83bc7b187110b7d22fae5f93bd355dd99557e656259d'" \
    "$helper" || fail 'the pre-alpha 1 KWin script fingerprint was not retained'
grep -Fq 'legacy-pre-alpha-3|legacy-pre-alpha-2|legacy-pre-alpha-1|legacy-0.1.0' \
    "$helper" || fail 'reviewed legacy KWin packages are not accepted by transaction recovery'

grep -Fq "overcrow_hypr_timeout_program='/usr/bin/timeout'" "$library" ||
    fail 'the Hyprland library does not use the fixed timeout program'
# shellcheck disable=SC2016 # Verify literal variables in the library source.
grep -Fq '"$overcrow_hypr_timeout_program" --signal=TERM --kill-after=1s 2s "$@"' \
    "$library" || fail 'the Hyprland command bound is not explicit'

if rg -n 'overcrow-supervise|supervisor_program|source_parent|source_helper' \
        "$helper" "$library" > /dev/null; then
    fail 'a compiled-supervisor or source-layout path remains'
fi
