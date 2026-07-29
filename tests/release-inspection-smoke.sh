#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
inspector="$project_root/packaging/release/inspect.sh"
version=0.1.0-pre-alpha.1
arch_version=0.1.0prealpha1
package_name="overcrow-bin-$arch_version-1-x86_64.pkg.tar.zst"
deb_version=0.1.0~pre.alpha.1-1
deb_name="overcrow_${deb_version}_amd64.deb"
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

build_deb() {
    destination=$1
    architecture=$2
    deb_root="$tmpdir/deb-$architecture"
    rm -rf -- "$deb_root"
    mkdir -p "$deb_root/control" "$deb_root/data/usr/bin"
    printf '%s\n' \
        'Package: overcrow' \
        "Version: $deb_version" \
        "Architecture: $architecture" \
        'Maintainer: Valhallab <contact@valhallab.com>' \
        'Description: test package' > "$deb_root/control/control"
    printf '2.0\n' > "$deb_root/debian-binary"
    tar -C "$deb_root/control" -cJf "$deb_root/control.tar.xz" ./control
    tar -C "$deb_root/data" -cJf "$deb_root/data.tar.xz" .
    (
        cd "$deb_root"
        ar r "$destination" debian-binary control.tar.xz data.tar.xz \
            >/dev/null 2>&1
    )
}

source_dir="$tmpdir/source"
mkdir -p "$source_dir"
build_package "$source_dir/$package_name" yes
build_deb "$source_dir/$deb_name" amd64
"$inspector" "$version" "$source_dir"

invalid_dir="$tmpdir/invalid"
mkdir -p "$invalid_dir"
build_package "$invalid_dir/$package_name" no
cp "$source_dir/$deb_name" "$invalid_dir/$deb_name"
if "$inspector" "$version" "$invalid_dir" \
        > "$tmpdir/invalid.out" 2> "$tmpdir/invalid.err"; then
    printf '%s\n' 'inspector accepted an incomplete package' >&2
    exit 1
fi

wrong_arch_dir="$tmpdir/wrong-arch"
mkdir -p "$wrong_arch_dir"
build_package "$wrong_arch_dir/$package_name" yes
build_deb "$wrong_arch_dir/$deb_name" arm64
if "$inspector" "$version" "$wrong_arch_dir" \
        > "$tmpdir/wrong-arch.out" 2> "$tmpdir/wrong-arch.err"; then
    printf '%s\n' 'inspector accepted a wrong-architecture DEB' >&2
    exit 1
fi

printf '%s\n' 'Release inspection smoke test passed'
