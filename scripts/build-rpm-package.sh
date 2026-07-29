#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
# The source path is derived from this checked-in script's physical root.
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"
dist_dir="$project_root/dist"

package_id=$(cd "$project_root" && cargo pkgid -p overcrow-control)
version=${package_id##*#}
if ! overcrow_version_is_valid "$version"; then
    printf '%s\n' "error: invalid workspace version: $version" >&2
    exit 1
fi
if ! rpm_version=$(overcrow_rpm_version "$version"); then
    printf '%s\n' "error: cannot normalize RPM version: $version" >&2
    exit 1
fi
if ! rpm_artifact_version=$(overcrow_rpm_artifact_version "$version"); then
    printf '%s\n' "error: cannot normalize RPM artifact version: $version" >&2
    exit 1
fi

if [ "$(id -u)" -eq 0 ]; then
    printf '%s\n' 'error: build the package as a regular desktop user' >&2
    exit 1
fi

for program in cargo cmp diff find install node npm readelf rpm rpmbuild \
        sha256sum sort tar zstd; do
    command -v "$program" >/dev/null 2>&1 || {
        printf '%s\n' "error: required build tool is unavailable: $program" >&2
        exit 1
    }
done

fedora_version=$(rpm --eval '%{fedora}')
case $fedora_version in
    42) ;;
    *)
        printf '%s\n' \
            "error: this initial RPM target requires Fedora 42, got: $fedora_version" >&2
        exit 1
        ;;
esac

mkdir -p "$dist_dir"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-rpm-package.XXXXXX")
published_work=
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    [ -z "$published_work" ] || rm -f -- "$published_work"
    rm -rf -- "$work_dir"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

publish_artifact() {
    source_path=$1
    artifact=$2
    published_work=$(mktemp "$dist_dir/.overcrow-rpm-package.XXXXXX")
    install -m 0644 "$source_path" "$published_work"
    mv -T -f -- "$published_work" "$artifact"
    published_work=
    printf '\n%s\n' "RPM package ready: $artifact"
}

: "${SOURCE_DATE_EPOCH:=$(date +%s)}"
case $SOURCE_DATE_EPOCH in
    ''|*[!0-9]*)
        printf '%s\n' 'error: SOURCE_DATE_EPOCH must be a non-negative integer' >&2
        exit 1
        ;;
esac
export SOURCE_DATE_EPOCH

printf '%s\n' "Building PlayerVox OverCrow $version for Fedora $fedora_version..."
cd "$project_root"
(
    cd "$project_root/crates/overcrow-control-ui"
    npm ci --ignore-scripts --no-audit --no-fund
    npm run build
)
remap_flag="--remap-path-prefix=$project_root=/usr/src/overcrow"
if [ -n "${RUSTFLAGS:-}" ]; then
    RUSTFLAGS="$RUSTFLAGS $remap_flag"
else
    RUSTFLAGS=$remap_flag
fi
export RUSTFLAGS
cargo fetch --locked
cargo build --workspace --release --locked

notices="$work_dir/THIRD_PARTY_LICENSES.md"
"$project_root/scripts/generate-third-party-notices.sh" "$notices"

stage="$work_dir/stage"
"$project_root/packaging/release/stage.sh" "$stage" "$notices"
(
    cd "$stage"
    find usr -type f -print | LC_ALL=C sort > "$work_dir/actual-manifest.txt"
)
if ! cmp -s "$project_root/packaging/release/manifest.txt" \
        "$work_dir/actual-manifest.txt"; then
    printf '%s\n' 'error: staged package does not match packaging/release/manifest.txt' >&2
    diff -u "$project_root/packaging/release/manifest.txt" \
        "$work_dir/actual-manifest.txt" >&2 || true
    exit 1
fi

rpm_root="$work_dir/rpmbuild"
mkdir -p \
    "$rpm_root/BUILD" \
    "$rpm_root/BUILDROOT" \
    "$rpm_root/RPMS" \
    "$rpm_root/SOURCES" \
    "$rpm_root/SPECS" \
    "$rpm_root/SRPMS"
bundle="$rpm_root/SOURCES/overcrow-$version-x86_64-linux.tar.zst"
"$project_root/packaging/release/archive.sh" "$version" "$stage" "$bundle"
spec="$rpm_root/SPECS/overcrow.spec"
"$project_root/packaging/rpm/render-spec.sh" "$version" "$bundle" "$spec"

rpmbuild -bb \
    --define "_topdir $rpm_root" \
    --define "_sourcedir $rpm_root/SOURCES" \
    --define "dist .fc$fedora_version" \
    "$spec"

built_rpm="$rpm_root/RPMS/x86_64/overcrow-$rpm_version-1.fc$fedora_version.x86_64.rpm"
if [ ! -f "$built_rpm" ] || [ -L "$built_rpm" ]; then
    printf '%s\n' 'error: rpmbuild did not produce the expected single RPM' >&2
    exit 1
fi
set -- "$rpm_root"/RPMS/*/*.rpm
if [ "$#" -ne 1 ] || [ "$1" != "$built_rpm" ]; then
    printf '%s\n' 'error: rpmbuild produced an unexpected RPM set' >&2
    exit 1
fi

identity=$(rpm -qp --qf '%{NAME}|%{VERSION}|%{RELEASE}|%{ARCH}\n' "$built_rpm")
expected_identity="overcrow|$rpm_version|1.fc$fedora_version|x86_64"
if [ "$identity" != "$expected_identity" ]; then
    printf '%s\n' "error: unexpected RPM identity: $identity" >&2
    exit 1
fi

rpm -qpl "$built_rpm" > "$work_dir/package-paths.txt"
rpm -qp --qf '[%{FILEMODES:perms} %{FILENAMES}\n]' "$built_rpm" \
    > "$work_dir/package-files.txt"
awk '
    $1 !~ /^d/ {
        path = $2
        sub("^/", "", path)
        print path
    }
' "$work_dir/package-files.txt" | LC_ALL=C sort > "$work_dir/package-manifest.txt"
if ! cmp -s "$project_root/packaging/release/manifest.txt" \
        "$work_dir/package-manifest.txt"; then
    printf '%s\n' 'error: RPM payload does not match the release manifest' >&2
    diff -u "$project_root/packaging/release/manifest.txt" \
        "$work_dir/package-manifest.txt" >&2 || true
    exit 1
fi
if awk '
    $1 ~ /^l/ || substr($1, 6, 1) == "w" || substr($1, 9, 1) == "w" {
        exit 42
    }
' "$work_dir/package-files.txt"; then
    :
else
    printf '%s\n' 'error: RPM contains a symlink or group/world-writable payload' >&2
    exit 1
fi

rpm -qpR "$built_rpm" > "$work_dir/package-requires.txt"
for requirement in systemd xdg-desktop-portal; do
    grep -Fqx "$requirement" "$work_dir/package-requires.txt" || {
        printf '%s\n' "error: RPM requirement is missing: $requirement" >&2
        exit 1
    }
done

rpm -qp --scripts "$built_rpm" > "$work_dir/package-scripts.txt"
grep -Fq 'PlayerVox OverCrow was installed inertly.' \
    "$work_dir/package-scripts.txt" || {
    printf '%s\n' 'error: expected inert RPM scriptlet is missing' >&2
    exit 1
}
if grep -Eq 'systemctl|rpm-ostree|kpackagetool|qdbus|hyprctl' \
        "$work_dir/package-scripts.txt"; then
    printf '%s\n' 'error: RPM scriptlet can mutate runtime or compositor state' >&2
    exit 1
fi

artifact="$dist_dir/overcrow-$rpm_artifact_version-1.fc$fedora_version.x86_64.rpm"
publish_artifact "$built_rpm" "$artifact"
printf '%s\n' 'Nothing was installed or started.'
