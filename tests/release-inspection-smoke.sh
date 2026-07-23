#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
inspector="$project_root/packaging/release/inspect.sh"
version=0.1.0-pre-alpha.1
arch_version=0.1.0prealpha1
package_name="overcrow-bin-$arch_version-1-x86_64.pkg.tar.zst"
tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-release-inspection.XXXXXX")

cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    rm -rf -- "$tmpdir"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

build_package() {
    destination=$1
    include_overlay=$2
    payload="$tmpdir/payload"
    rm -rf -- "$payload"
    mkdir -p "$payload/usr/bin" "$payload/usr/lib/overcrow"
    : > "$payload/usr/bin/overcrow-control"
    : > "$payload/usr/bin/overcrow-core"
    if test "$include_overlay" = yes; then
        : > "$payload/usr/bin/overcrow-overlay"
    fi
    : > "$payload/usr/lib/overcrow/overcrow-integrate"
    printf '%s\n' \
        'pkgname = overcrow-bin' \
        "pkgver = $arch_version-1" \
        'arch = x86_64' > "$payload/.PKGINFO"
    bsdtar -caf "$destination" -C "$payload" .PKGINFO usr
}

source_dir="$tmpdir/source"
mkdir -p "$source_dir"
build_package "$source_dir/$package_name" yes
"$inspector" "$version" "$source_dir"

invalid_dir="$tmpdir/invalid"
mkdir -p "$invalid_dir"
build_package "$invalid_dir/$package_name" no
if "$inspector" "$version" "$invalid_dir" \
        > "$tmpdir/invalid.out" 2> "$tmpdir/invalid.err"; then
    printf '%s\n' 'inspector accepted an incomplete package' >&2
    exit 1
fi

printf '%s\n' 'Release inspection smoke test passed'
