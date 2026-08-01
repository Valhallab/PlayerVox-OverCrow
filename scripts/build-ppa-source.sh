#!/bin/sh
set -eu
umask 022

usage() {
    printf '%s\n' 'usage: build-ppa-source.sh [--unsigned] [PPA_REVISION]' >&2
    exit 2
}

unsigned=false
if test "${1:-}" = --unsigned; then
    unsigned=true
    shift
fi
case $# in
    0) ppa_revision=1 ;;
    1) ppa_revision=$1 ;;
    *) usage ;;
esac

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
# The source path is derived from this checked-in script's physical root.
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"

package_id=$(cd "$project_root" && cargo pkgid -p overcrow-control)
version=${package_id##*#}
ppa_version=$(overcrow_ppa_package_version "$version" "$ppa_revision") || {
    printf '%s\n' 'error: invalid release version or PPA revision' >&2
    exit 2
}
ppa_upstream_version=$(overcrow_ppa_upstream_version "$version" "$ppa_revision") || {
    printf '%s\n' 'error: invalid PPA upstream version' >&2
    exit 2
}

if test "$(id -u)" -eq 0; then
    printf '%s\n' 'error: build the source package as a regular user' >&2
    exit 1
fi
if test "$(uname -m)" != x86_64; then
    printf '%s\n' 'error: the initial PPA source target requires x86_64' >&2
    exit 1
fi

required_programs='awk cargo cmp cp date dh dpkg-buildpackage du find git gpg grep install mktemp npm sed sort tar wc xz'
if test "$unsigned" = false; then
    required_programs="$required_programs debsign"
fi
for program in $required_programs; do
    command -v "$program" >/dev/null 2>&1 || {
        printf '%s\n' "error: required source build tool is unavailable: $program" >&2
        exit 1
    }
done

git_root=$(cd "$project_root" && git rev-parse --show-toplevel)
git_root=$(CDPATH='' cd -- "$git_root" && pwd -P)
if test "$git_root" != "$project_root"; then
    printf '%s\n' 'error: source builder must run from the repository root' >&2
    exit 1
fi
if ! (cd "$project_root" && git diff --quiet --no-ext-diff &&
        git diff --cached --quiet --no-ext-diff); then
    printf '%s\n' 'error: source package requires a clean tracked checkout' >&2
    exit 1
fi
untracked=$(cd "$project_root" && git ls-files --others --exclude-standard)
if test -n "$untracked"; then
    printf '%s\n' 'error: source package requires no untracked files' >&2
    exit 1
fi

source_date_epoch=$(cd "$project_root" && git show -s --format=%ct HEAD)
case $source_date_epoch in
    ''|*[!0-9]*)
        printf '%s\n' 'error: source commit timestamp is invalid' >&2
        exit 1
        ;;
esac
export SOURCE_DATE_EPOCH="$source_date_epoch"

dist_dir="$project_root/dist"
output_parent="$dist_dir/ppa"
install -d -m 0755 "$output_parent"
destination="$output_parent/$ppa_version"
if test -e "$destination" || test -L "$destination"; then
    printf '%s\n' "error: PPA source output already exists: $destination" >&2
    exit 1
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-ppa-source.XXXXXX")
chmod 0700 "$work_dir"
published_work=
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    if test -n "$published_work"; then
        rm -rf -- "$published_work"
    fi
    rm -rf -- "$work_dir"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

source_name="overcrow-$ppa_upstream_version"
source_root="$work_dir/$source_name"
install -d -m 0755 "$source_root"
(cd "$project_root" && git archive --format=tar HEAD) |
    tar -xf - -C "$source_root"

printf '%s\n' "Preparing PlayerVox OverCrow $version source for Launchpad..."
(
    cd "$project_root/crates/overcrow-control-ui"
    npm ci --ignore-scripts --no-audit --no-fund
    npm run build
)
frontend_dist="$project_root/crates/overcrow-control-ui/dist"
test -s "$frontend_dist/index.html" || {
    printf '%s\n' 'error: frontend build did not produce its entry point' >&2
    exit 1
}
if find "$frontend_dist" -type l -print -quit | grep -q .; then
    printf '%s\n' 'error: frontend build contains a symlink' >&2
    exit 1
fi
frontend_files=$(find "$frontend_dist" -type f -print | wc -l)
frontend_kib=$(du -sk "$frontend_dist" | awk '{ print $1 }')
if test "$frontend_files" -gt 512 || test "$frontend_kib" -gt 32768; then
    printf '%s\n' 'error: frontend build exceeds source-package bounds' >&2
    exit 1
fi
source_frontend="$source_root/crates/overcrow-control-ui/dist"
install -d -m 0755 "$source_frontend"
cp -a "$frontend_dist/." "$source_frontend/"

install -d -m 0755 "$source_root/.cargo"
vendor_config="$work_dir/vendor-config.toml"
(
    cd "$source_root"
    cargo vendor --locked --versioned-dirs vendor
) > "$vendor_config"
grep -Fq 'replace-with = "vendored-sources"' "$vendor_config" || {
    printf '%s\n' 'error: Cargo did not produce an offline source replacement' >&2
    exit 1
}
if ! grep -Fq 'directory = "vendor"' "$vendor_config" ||
        grep -Fq "$work_dir" "$vendor_config" ||
        grep -Fq "$project_root" "$vendor_config"; then
    printf '%s\n' 'error: Cargo vendor configuration is not relocatable' >&2
    exit 1
fi
if find "$source_root/vendor" -type l -print -quit | grep -q .; then
    printf '%s\n' 'error: vendored Cargo sources contain a symlink' >&2
    exit 1
fi
vendor_files=$(find "$source_root/vendor" -type f -print | wc -l)
vendor_kib=$(du -sk "$source_root/vendor" | awk '{ print $1 }')
if test "$vendor_files" -gt 100000 || test "$vendor_kib" -gt 2097152; then
    printf '%s\n' 'error: vendored Cargo sources exceed source-package bounds' >&2
    exit 1
fi
install -m 0644 "$vendor_config" "$source_root/.cargo/config.toml"

notices="$work_dir/THIRD_PARTY_LICENSES.md"
"$project_root/scripts/generate-third-party-notices.sh" "$notices"
test -s "$notices" || {
    printf '%s\n' 'error: third-party notices are missing' >&2
    exit 1
}

find "$source_root" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
orig_tar="$work_dir/overcrow_${ppa_upstream_version}.orig.tar.xz"
tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 \
    --numeric-owner --format=gnu -cJf "$orig_tar" -C "$work_dir" "$source_name"
test -s "$orig_tar" || {
    printf '%s\n' 'error: original source archive is missing' >&2
    exit 1
}

install -d -m 0755 "$source_root/debian/source"
install -m 0644 "$project_root/packaging/ppa/debian/control" \
    "$source_root/debian/control"
install -m 0755 "$project_root/packaging/ppa/debian/rules" \
    "$source_root/debian/rules"
install -m 0644 "$project_root/packaging/ppa/debian/source/format" \
    "$source_root/debian/source/format"
install -m 0644 "$project_root/packaging/ppa/debian/source/options" \
    "$source_root/debian/source/options"
install -m 0644 "$project_root/packaging/deb/copyright" \
    "$source_root/debian/copyright"
install -m 0644 "$notices" "$source_root/debian/THIRD_PARTY_LICENSES.md"

changelog_date=$(LC_ALL=C date -u -R -d "@$SOURCE_DATE_EPOCH")
sed \
    -e "s|@PPA_VERSION@|$ppa_version|g" \
    -e "s|@UPSTREAM_VERSION@|$version|g" \
    -e "s|@CHANGELOG_DATE@|$changelog_date|g" \
    "$project_root/packaging/ppa/debian/changelog.in" \
    > "$source_root/debian/changelog"
chmod 0644 "$source_root/debian/changelog"

(
    cd "$source_root"
    # Source preparation does not require the target build dependencies. The
    # Launchpad builder resolves and enforces them before compiling the package.
    dpkg-buildpackage -S -sa -us -uc -d
)

changes="$work_dir/overcrow_${ppa_version}_source.changes"
dsc="$work_dir/overcrow_${ppa_version}.dsc"
debian_tar="$work_dir/overcrow_${ppa_version}.debian.tar.xz"
buildinfo="$work_dir/overcrow_${ppa_version}_source.buildinfo"
for artifact in "$orig_tar" "$dsc" "$debian_tar" "$buildinfo" "$changes"; do
    if ! test -f "$artifact" || test -L "$artifact" || ! test -s "$artifact"; then
        printf '%s\n' "error: source build artifact is invalid: $(basename "$artifact")" >&2
        exit 1
    fi
done

signing_fingerprint=6425BB0DBE7933E086EE420B2789BF4BF0C19541
if test "$unsigned" = false; then
    debsign -k"$signing_fingerprint" "$changes"
    for signed_file in "$dsc" "$buildinfo" "$changes"; do
        verification=$(gpg --batch --status-fd 1 --verify "$signed_file" 2>/dev/null) || {
            printf '%s\n' "error: source signature is invalid: $(basename "$signed_file")" >&2
            exit 1
        }
        if ! printf '%s\n' "$verification" |
                grep -Fq "[GNUPG:] VALIDSIG $signing_fingerprint "; then
            printf '%s\n' "error: source artifact has the wrong signer: $(basename "$signed_file")" >&2
            exit 1
        fi
    done
fi

published_work=$(mktemp -d "$output_parent/.overcrow-ppa-source.XXXXXX")
chmod 0700 "$published_work"
for artifact in "$orig_tar" "$dsc" "$debian_tar" "$buildinfo" "$changes"; do
    install -m 0644 "$artifact" "$published_work/$(basename "$artifact")"
done
if ! mv -T -n -- "$published_work" "$destination"; then
    printf '%s\n' 'error: could not publish the PPA source output' >&2
    exit 1
fi
published_work=

printf '\n%s\n' "PPA source package ready: $destination"
printf '%s\n' "Upload file: $destination/$(basename "$changes")"
if test "$unsigned" = true; then
    printf '%s\n' 'The source set is intentionally unsigned and cannot be uploaded.'
fi
printf '%s\n' 'Nothing was installed, started, or uploaded.'
