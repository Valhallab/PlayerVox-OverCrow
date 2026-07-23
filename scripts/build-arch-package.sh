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
if ! arch_version=$(overcrow_arch_version "$version"); then
    printf '%s\n' "error: cannot normalize Arch version: $version" >&2
    exit 1
fi

if [ "$(id -u)" -eq 0 ]; then
    printf '%s\n' 'error: build the package as a regular desktop user' >&2
    exit 1
fi

for program in cargo makepkg node npm zstd readelf; do
    command -v "$program" >/dev/null 2>&1 || {
        printf '%s\n' "error: required build tool is unavailable: $program" >&2
        exit 1
    }
done

mkdir -p "$dist_dir"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-package.XXXXXX")
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
    published_work=$(mktemp "$dist_dir/.overcrow-package.XXXXXX")
    install -m 0644 "$source_path" "$published_work"
    mv -T -f -- "$published_work" "$artifact"
    published_work=
    printf '\n%s\n' "Package ready: $artifact"
}

: "${SOURCE_DATE_EPOCH:=$(date +%s)}"
case $SOURCE_DATE_EPOCH in
    ''|*[!0-9]*)
        printf '%s\n' 'error: SOURCE_DATE_EPOCH must be a non-negative integer' >&2
        exit 1
        ;;
esac
export SOURCE_DATE_EPOCH

printf '%s\n' "Building OverCrow $version..."
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
    printf '%s\n' 'error: staged package does not match the release manifest' >&2
    diff -u "$project_root/packaging/release/manifest.txt" \
        "$work_dir/actual-manifest.txt" >&2 || true
    exit 1
fi

package_dir="$work_dir/package"
packages_dir="$work_dir/packages"
mkdir -p "$package_dir" "$packages_dir"
bundle="$package_dir/overcrow-$version-x86_64-linux.tar.zst"
"$project_root/packaging/release/archive.sh" "$version" "$stage" "$bundle"
"$project_root/packaging/arch/render-pkgbuild.sh" \
    "$version" "$bundle" "$package_dir/PKGBUILD"
install -m 0644 "$project_root/packaging/arch/overcrow.install" \
    "$package_dir/overcrow.install"

(
    cd "$package_dir"
    PKGDEST="$packages_dir" makepkg \
        --clean --cleanbuild --force --noconfirm --nodeps
)

set -- "$packages_dir"/overcrow-bin-"$arch_version"-1-x86_64.pkg.tar.*
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
    printf '%s\n' 'error: makepkg did not produce overcrow-bin' >&2
    exit 1
fi

artifact="$dist_dir/${1##*/}"
publish_artifact "$1" "$artifact"
printf '%s\n' 'Nothing was installed or started.'
