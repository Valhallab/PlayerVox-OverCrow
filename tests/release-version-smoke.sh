#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
version_helper="$project_root/scripts/lib/release-version.sh"
version=0.1.0-pre-alpha.1

# The source path is derived from this checked-in script's physical root.
# shellcheck disable=SC1090,SC1091
. "$version_helper"

overcrow_version_is_valid 0.1.0
overcrow_version_is_valid "$version"

for invalid in \
        01.1.0 \
        0.01.0 \
        0.1.00 \
        0.1 \
        0.1.0- \
        0.1.0+metadata \
        0.1.0/escape \
        '0.1.0;touch injected'; do
    if overcrow_version_is_valid "$invalid"; then
        printf '%s\n' "accepted invalid release version: $invalid" >&2
        exit 1
    fi
done

test "$(overcrow_arch_version 0.1.0)" = 0.1.0
test "$(overcrow_arch_version "$version")" = 0.1.0prealpha1
test "$(vercmp 0.1.0prealpha1 0.1.0)" -lt 0

grep -Fqx "version = \"$version\"" "$project_root/Cargo.toml"
grep -Fq "\"version\": \"$version\"" \
    "$project_root/crates/overcrow-control-ui/package.json"
grep -Fq "\"version\": \"$version\"" \
    "$project_root/crates/overcrow-control-ui/package-lock.json"
grep -Fq "\"version\": \"$version\"" \
    "$project_root/crates/overcrow-control-ui/src-tauri/tauri.conf.json"
grep -Fq "\"Version\": \"$version\"" \
    "$project_root/integrations/kwin/metadata.json"
grep -Fq "<release version=\"$version\" date=\"2026-07-23\">" \
    "$project_root/packaging/metainfo/com.playervox.OverCrow.metainfo.xml"
printf '%s\n' 'Release version smoke test passed'
