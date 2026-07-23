#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

test -f deny.toml
test -f about.toml
test -f packaging/licenses/third-party.hbs
grep -Fq '"MPL-2.0"' about.toml
grep -Fq '"MPL-2.0"' deny.toml
grep -Fqx 'SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007' \
    assets/branding/NotoSans-OFL.txt
grep -Fq 'Noto Sans Regular' assets/branding/NotoSans-OFL.txt
grep -Fq 'Noto Sans Black Italic' assets/branding/NotoSans-OFL.txt
test -x scripts/check-dependency-licenses.sh
test -x scripts/generate-third-party-notices.sh

sh -n scripts/check-dependency-licenses.sh
sh -n scripts/generate-third-party-notices.sh

scripts/check-dependency-licenses.sh

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-licenses.XXXXXX")
trap 'rm -rf -- "$tmpdir"' EXIT HUP INT TERM
notices="$tmpdir/THIRD_PARTY_LICENSES.md"
scripts/generate-third-party-notices.sh "$notices"

test -s "$notices"
if grep -Fq '&quot;' "$notices"; then
    printf '%s\n' 'third-party notices contain HTML-escaped license text' >&2
    exit 1
fi
grep -Fq '"License" shall mean the terms and conditions' "$notices"
grep -Fqx '# OverCrow third-party software notices' "$notices"
grep -Fq 'serde ' "$notices"
if grep -Fq "$project_root" "$notices"; then
    printf '%s\n' 'third-party notices contain the private workspace path' >&2
    exit 1
fi
