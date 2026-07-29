#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
renderer="$root/packaging/rpm/render-spec.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-rpm-smoke.XXXXXX")

cleanup() {
    rm -rf -- "$work"
}
trap cleanup EXIT HUP INT TERM

fail() {
    printf '%s\n' "RPM package smoke test failed: $1" >&2
    exit 1
}

expect_failure() {
    if "$@" >/dev/null 2>&1; then
        fail "command unexpectedly succeeded: $*"
    fi
}

[ -x "$renderer" ] || fail 'RPM spec renderer is unavailable'

bundle="$work/overcrow-0.1.0-pre-alpha.3-x86_64-linux.tar.zst"
printf '%s\n' 'bounded test payload' > "$bundle"
spec="$work/overcrow.spec"

"$renderer" '0.1.0-pre-alpha.3' "$bundle" "$spec"

grep -Fq 'Name:           overcrow' "$spec" ||
    fail 'package name is not fixed'
grep -Fq 'Version:        0.1.0~pre_alpha.3' "$spec" ||
    fail 'pre-release version was not normalized for RPM'
grep -Fq 'Release:        1%{?dist}' "$spec" ||
    fail 'release does not use the distribution suffix'
grep -Fq '%global debug_package %{nil}' "$spec" ||
    fail 'a user-facing debug subpackage remains enabled'
grep -Fq 'PlayerVox OverCrow was installed inertly.' "$spec" ||
    fail 'inert installation copy is missing'
grep -Fq '%{_bindir}/overcrow-control' "$spec" ||
    fail 'the complete application payload is not listed'
grep -Fq "Source0:        ${bundle##*/}" "$spec" ||
    fail 'source bundle basename is not pinned'
grep -Fq "Source1:        ${bundle##*/}.sha256" "$spec" ||
    fail 'source checksum basename is not pinned'

digest=$(sha256sum "$bundle")
digest=${digest%% *}
grep -Fq "$digest  ${bundle##*/}" "$work/${bundle##*/}.sha256" ||
    fail 'source checksum sidecar is incorrect'

expect_failure "$renderer" 'not-a-version' "$bundle" "$work/invalid.spec"
expect_failure "$renderer" '0.1.0-pre-alpha.3' relative.tar.zst "$work/relative.spec"
ln -s "$bundle" "$work/bundle-link"
expect_failure "$renderer" '0.1.0-pre-alpha.3' "$work/bundle-link" "$work/link.spec"
expect_failure "$renderer" '0.1.0-pre-alpha.3' "$bundle" "$spec"
expect_failure "$renderer" '0.1.0-pre-alpha.3' "$bundle" "$work/extra.spec" extra

printf '%s\n' 'RPM package smoke test passed'
