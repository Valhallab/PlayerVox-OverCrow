#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
builder="$root/scripts/build-ppa-source.sh"
debian="$root/packaging/ppa/debian"

fail() {
    printf '%s\n' "PPA source package smoke test failed: $1" >&2
    exit 1
}

test -x "$builder" || fail 'source builder is unavailable'
sh -n "$builder"
test -f "$debian/control" || fail 'Debian source control is unavailable'
test -x "$debian/rules" || fail 'Debian source rules are unavailable'
test "$(cat "$debian/source/format")" = '3.0 (quilt)' ||
    fail 'source format is not 3.0 (quilt)'

grep -Fq 'Source: overcrow' "$debian/control" || fail 'source identity is wrong'
grep -Fq 'Package: overcrow' "$debian/control" || fail 'binary identity is wrong'
grep -Fq 'Architecture: amd64' "$debian/control" ||
    fail 'binary architecture is not the reviewed amd64 target'
grep -Fq 'Rules-Requires-Root: no' "$debian/control" ||
    fail 'source build does not explicitly remain unprivileged'
grep -Fq 'cargo-1.92' "$debian/control" || fail 'Cargo 1.92 is not required'
grep -Fq 'rustc-1.92' "$debian/control" || fail 'Rust 1.92 is not required'
grep -Eq '^overcrow \(@PPA_VERSION@\) noble; urgency=' \
    "$debian/changelog.in" || fail 'source suite is not Ubuntu Noble'

grep -Fq '/usr/bin/cargo-1.92' "$debian/rules" ||
    fail 'versioned Cargo is not selected'
grep -Fq '/usr/bin/rustc-1.92' "$debian/rules" ||
    fail 'versioned rustc is not selected'
grep -Fq -- '--offline' "$debian/rules" || fail 'Cargo build is not offline'
grep -Fq 'packaging/release/stage.sh' "$debian/rules" ||
    fail 'canonical release staging is not reused'
grep -Fq 'packaging/release/manifest.txt' "$debian/rules" ||
    fail 'canonical payload manifest is not verified'
grep -Fq 'dh_clean -Xvendor/' "$debian/rules" ||
    fail 'Debhelper cleanup can delete checksummed vendored files'
grep -Fq 'override_dh_installsystemduser:' "$debian/rules" ||
    fail 'Debhelper can enable OverCrow user services during installation'
grep -Fq 'Runtime lifecycle remains owned by the Control Center.' \
    "$debian/rules" ||
    fail 'the inert user-service override is not documented'
if grep -Eq '^[[:space:]]+dh_installsystemduser([[:space:]]|$)' \
        "$debian/rules"; then
    fail 'the package invokes Debhelper user-service integration'
fi

# These are intentional literal command shapes read from the target script.
# shellcheck disable=SC2016
for contract in \
        'umask 022' \
        'git archive --format=tar HEAD' \
        'npm ci --ignore-scripts --no-audit --no-fund' \
        'npm run build' \
        'cargo vendor --locked --versioned-dirs vendor' \
        'directory = "vendor"' \
        'ppa_upstream_version=$(overcrow_ppa_upstream_version "$version" "$ppa_revision")' \
        'orig_tar="$work_dir/overcrow_${ppa_upstream_version}.orig.tar.xz"' \
        'scripts/generate-third-party-notices.sh' \
        'dpkg-buildpackage -S -sa -us -uc -d' \
        'debsign -k' \
        'gpg --batch --status-fd 1 --verify' \
        '6425BB0DBE7933E086EE420B2789BF4BF0C19541' \
        'output_parent="$dist_dir/ppa"'; do
    grep -Fq -- "$contract" "$builder" ||
        fail "source builder is missing contract: $contract"
done

if grep -Eiq '(^|[[:space:]])(sudo|apt|apt-get|dput|systemctl|curl|wget)([[:space:]]|$)|git[[:space:]]+push' \
        "$builder"; then
    fail 'source builder installs, uploads, or performs network publication'
fi
if grep -En 'rm[[:space:]].*(dist_dir|/dist|output_parent)' "$builder" >/dev/null; then
    fail 'source builder can delete unrelated distribution artifacts'
fi

printf '%s\n' 'PPA source package smoke test passed'
