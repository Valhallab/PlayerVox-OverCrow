#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
library="$repo_root/scripts/lib/hyprland-config.sh"

fail() {
    printf '%s\n' "Hyprland config smoke test failed: $1" >&2
    exit 1
}

sh -n "$library"
grep -Fq "overcrow_hypr_timeout_program='/usr/bin/timeout'" "$library" || \
    fail 'the fixed timeout program is missing'
# shellcheck disable=SC2016 # Verify literal variables in the library source.
grep -Fq '"$overcrow_hypr_timeout_program" --signal=TERM --kill-after=1s 2s "$@"' \
    "$library" || fail 'bounded commands do not use the fixed timeout'
if grep -Eq 'overcrow-supervise|supervisor_program' "$library"; then
    fail 'legacy supervisor code remains'
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

# Some development sandboxes remap root-owned files to uid 65534. Adapt only
# the test copy; the packaged library keeps its root-owned trust requirement.
test_library="$tmpdir/hyprland-config.sh"
system_owner=$(stat -c '%u' /usr/bin)
sed "s/^overcrow_hypr_system_program_owner=0$/overcrow_hypr_system_program_owner=$system_owner/" \
    "$library" > "$test_library"
# The library has no side effects when sourced.
# shellcheck disable=SC1090
. "$test_library"

source_file="$tmpdir/source.conf"
stripped_file="$tmpdir/stripped.conf"
expected_file="$tmpdir/expected.conf"
printf '%s\n' \
    'before' \
    '# BEGIN OVERCROW MANAGED' \
    'source = /tmp/overcrow.conf' \
    '# END OVERCROW MANAGED' \
    'after' > "$source_file"
printf '%s\n' 'before' 'after' > "$expected_file"
overcrow_hypr_strip_block "$source_file" "$stripped_file" \
    '# BEGIN OVERCROW MANAGED' '# END OVERCROW MANAGED'
cmp -s "$stripped_file" "$expected_file" || fail 'managed block removal changed user content'

printf '%s\n' '# END OVERCROW MANAGED' > "$source_file"
if overcrow_hypr_strip_block "$source_file" "$stripped_file" \
    '# BEGIN OVERCROW MANAGED' '# END OVERCROW MANAGED'; then
    fail 'a malformed managed block was accepted'
fi

config_home="$tmpdir/config"
mkdir -p "$config_home/hypr"
printf '%s\n' '# user setting' > "$config_home/hypr/hyprland.conf"

# Avoid touching a running compositor while exercising the real file transaction.
# shellcheck disable=SC2034 # Consumed by functions from the sourced library.
overcrow_hyprctl_program='/usr/bin/overcrow-hyprctl-not-installed'
install_hyprland_config "$config_home" "$repo_root/integrations/hyprland"
hyprland_config_ready "$config_home" "$repo_root/integrations/hyprland" || \
    fail 'installed configuration is not reported ready'
grep -Fq '# user setting' "$config_home/hypr/hyprland.conf" || \
    fail 'installation removed existing user configuration'

remove_hyprland_config "$config_home"
if grep -Fq '# BEGIN OVERCROW MANAGED' "$config_home/hypr/hyprland.conf"; then
    fail 'removal left the managed block behind'
fi
[ ! -e "$config_home/hypr/overcrow.conf" ] || fail 'removal left the managed fragment behind'
grep -Fq '# user setting' "$config_home/hypr/hyprland.conf" || \
    fail 'removal changed existing user configuration'
