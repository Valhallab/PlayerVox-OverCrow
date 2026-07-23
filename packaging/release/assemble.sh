#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'usage: assemble.sh VERSION SOURCE_DIR OUTPUT_DIR' >&2
    exit 2
}

test "$#" -eq 3 || usage
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"

version=$1
source_dir=$2
output_dir=$3

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
case $output_dir in
    /*) ;;
    *) printf '%s\n' 'error: OUTPUT_DIR must be absolute' >&2; exit 2 ;;
esac
if ! test -d "$source_dir"; then
    printf '%s\n' 'error: SOURCE_DIR is not a directory' >&2
    exit 2
fi

output_parent=$(dirname -- "$output_dir")
if ! test -d "$output_parent"; then
    printf '%s\n' 'error: OUTPUT_DIR parent does not exist' >&2
    exit 2
fi
if test -e "$output_dir" || test -L "$output_dir"; then
    printf '%s\n' 'error: OUTPUT_DIR already exists' >&2
    exit 2
fi

artifact="overcrow-bin-$arch_version-1-x86_64.pkg.tar.zst"
source_path="$source_dir/$artifact"
if ! test -f "$source_path" || test -L "$source_path" ||
        ! test -s "$source_path"; then
    printf '%s\n' "error: invalid release artifact: $artifact" >&2
    exit 1
fi

working=
cleanup_working() {
    if test -n "$working" &&
            { test -e "$working" || test -L "$working"; }; then
        rm -rf -- "$working"
    fi
}
handle_exit() {
    status=$?
    trap - EXIT HUP INT TERM
    cleanup_working
    exit "$status"
}
handle_signal() {
    trap - EXIT HUP INT TERM
    cleanup_working || :
    exit 1
}
trap handle_exit EXIT
trap handle_signal HUP INT TERM

working=$(mktemp -d "$output_parent/.overcrow-release.XXXXXX")
chmod 0700 "$working"
install -m 0644 "$source_path" "$working/$artifact"

(
    cd "$working"
    sha256sum "$artifact" > SHA256SUMS
    chmod 0644 SHA256SUMS
    sha256sum -c SHA256SUMS >/dev/null
)

if test -e "$output_dir" || test -L "$output_dir"; then
    printf '%s\n' 'error: OUTPUT_DIR appeared before publication' >&2
    exit 1
fi
if ! mv -T -n -- "$working" "$output_dir"; then
    printf '%s\n' 'error: could not publish OUTPUT_DIR' >&2
    exit 1
fi
if test -e "$working" || test -L "$working"; then
    printf '%s\n' 'error: OUTPUT_DIR appeared during publication' >&2
    exit 1
fi
working=

printf '%s\n' "Release candidate ready: $output_dir"
printf '%s\n' 'Nothing was installed, tagged, uploaded, or started.'
