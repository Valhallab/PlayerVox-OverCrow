#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'usage: inspect.sh VERSION SOURCE_DIR' >&2
    exit 2
}

test "$#" -eq 2 || usage
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"

version=$1
source_dir=$2
if ! overcrow_version_is_valid "$version"; then
    printf '%s\n' 'error: invalid version' >&2
    exit 2
fi
if ! arch_version=$(overcrow_arch_version "$version"); then
    printf '%s\n' 'error: could not normalize the Arch version' >&2
    exit 2
fi
case $source_dir in
    /*) ;;
    *) printf '%s\n' 'error: SOURCE_DIR must be absolute' >&2; exit 2 ;;
esac
if ! test -d "$source_dir"; then
    printf '%s\n' 'error: SOURCE_DIR is not a directory' >&2
    exit 2
fi
command -v bsdtar >/dev/null 2>&1 || {
    printf '%s\n' 'error: required inspection tool is unavailable: bsdtar' >&2
    exit 1
}

package="$source_dir/overcrow-bin-$arch_version-1-x86_64.pkg.tar.zst"
if ! test -f "$package" || test -L "$package" || ! test -s "$package"; then
    printf '%s\n' 'error: invalid Arch package: overcrow-bin' >&2
    exit 1
fi
if ! package_info=$(bsdtar -xOf "$package" .PKGINFO); then
    printf '%s\n' 'error: cannot read Arch package metadata' >&2
    exit 1
fi
for expected in \
        'pkgname = overcrow-bin' \
        "pkgver = $arch_version-1" \
        'arch = x86_64'; do
    if test "$(printf '%s\n' "$package_info" | grep -Fxc "$expected")" -ne 1; then
        printf '%s\n' 'error: invalid Arch package metadata' >&2
        exit 1
    fi
done

if ! listing=$(bsdtar -tf "$package"); then
    printf '%s\n' 'error: cannot read Arch package contents' >&2
    exit 1
fi
for required in \
        usr/bin/overcrow-control \
        usr/bin/overcrow-core \
        usr/bin/overcrow-overlay \
        usr/lib/overcrow/overcrow-integrate; do
    if ! printf '%s\n' "$listing" | grep -Fqx "$required"; then
        printf '%s\n' "error: Arch package is missing $required" >&2
        exit 1
    fi
done

printf '%s\n' 'Release artifact metadata inspection passed'
