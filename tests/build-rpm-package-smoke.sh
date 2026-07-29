#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
builder="$root/scripts/build-rpm-package.sh"

fail() {
    printf '%s\n' "RPM build smoke test failed: $1" >&2
    exit 1
}

[ -x "$builder" ] || fail 'RPM build script is unavailable'
sh -n "$builder"

grep -Fq 'cargo fetch --locked' "$builder" ||
    fail 'the locked dependency cache is not prepared for offline notices'
grep -Fq 'cargo build --workspace --release --locked' "$builder" ||
    fail 'release build is not locked'
grep -Fq 'packaging/release/stage.sh' "$builder" ||
    fail 'canonical release staging is not reused'
grep -Fq 'packaging/release/manifest.txt' "$builder" ||
    fail 'staged payload is not checked against the manifest'
grep -Fq 'packaging/rpm/render-spec.sh' "$builder" ||
    fail 'reviewed RPM spec renderer is not used'
grep -Fq 'rpmbuild -bb' "$builder" ||
    fail 'binary RPM build is missing'
grep -Fq 'rpm -qp' "$builder" ||
    fail 'RPM identity is not inspected'
grep -Fq 'rpm -qpl' "$builder" ||
    fail 'RPM payload is not inspected'
grep -Fq 'rpm -qpR' "$builder" ||
    fail 'RPM dependencies are not inspected'
grep -Fq 'Nothing was installed or started.' "$builder" ||
    fail 'inert build result is not explicit'

if grep -En '(^|[[:space:]])(sudo|rpm-ostree|systemctl|curl|wget)([[:space:]]|$)|git[[:space:]]+push' \
        "$builder" >/dev/null; then
    fail 'build script contains an installation, network, or publication command'
fi
if grep -En 'rm[[:space:]].*(dist_dir|/dist)' "$builder" >/dev/null; then
    fail 'build script can delete unrelated distribution artifacts'
fi

printf '%s\n' 'RPM build smoke test passed'
