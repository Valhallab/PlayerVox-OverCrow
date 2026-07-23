#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

fail() {
    printf '%s\n' "systemd unit smoke test failed: $*" >&2
    exit 1
}

assert_exact_line() {
    line=$1
    file=$2
    grep -Fqx "$line" "$file" || fail "missing '$line' in $file"
}

assert_absent() {
    pattern=$1
    file=$2
    if grep -Fq "$pattern" "$file"; then
        fail "unexpected '$pattern' in $file"
    fi
}

core=packaging/systemd/overcrow-core.service.in
overlay=packaging/systemd/overcrow-overlay.service.in
hyprland=packaging/systemd/overcrow-hyprland.service.in

for template in "$core" "$overlay" "$hyprland"; do
    [ -f "$template" ] || fail "missing template $template"
    assert_absent '%h/.local/bin' "$template"
    assert_exact_line 'NoNewPrivileges=true' "$template"
    assert_exact_line 'UMask=0077' "$template"
    assert_exact_line 'LockPersonality=true' "$template"
    assert_exact_line 'RestrictSUIDSGID=true' "$template"
    assert_exact_line 'SystemCallArchitectures=native' "$template"
done

for legacy in \
    packaging/systemd/overcrow-core.service \
    packaging/systemd/overcrow-overlay.service \
    packaging/systemd/overcrow-hyprland.service
do
    [ ! -e "$legacy" ] || fail "legacy unit still exists: $legacy"
done

assert_exact_line 'ExecStart=@OVERCROW_BINDIR@/overcrow-core' "$core"
assert_exact_line '[Install]' "$core"
assert_exact_line 'WantedBy=default.target' "$core"

assert_exact_line 'ExecStart=@OVERCROW_BINDIR@/overcrow-overlay' "$overlay"
assert_exact_line 'Requires=overcrow-core.service' "$overlay"
assert_absent '[Install]' "$overlay"
assert_absent 'WantedBy=' "$overlay"

assert_exact_line 'ExecStart=@OVERCROW_BINDIR@/overcrow-hyprland' "$hyprland"
assert_exact_line 'ExecStopPost=@OVERCROW_BINDIR@/overcrow-hyprland --cleanup-focus-state' "$hyprland"
assert_exact_line 'Requires=overcrow-core.service' "$hyprland"
assert_absent '[Install]' "$hyprland"
assert_absent 'WantedBy=' "$hyprland"

printf '%s\n' 'systemd unit templates: ok'
