#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'usage: archive.sh VERSION STAGE OUTPUT' >&2
    exit 2
}

test "$#" -eq 3 || usage
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
# The source path is derived from this checked-in script's physical root.
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"
version=$1
stage=$2
output=$3

if ! overcrow_version_is_valid "$version"; then
    printf '%s\n' 'error: invalid version' >&2
    exit 2
fi
case $stage in
    /*) ;;
    *) printf '%s\n' 'error: STAGE must be absolute' >&2; exit 2 ;;
esac
case $output in
    /*) ;;
    *) printf '%s\n' 'error: OUTPUT must be absolute' >&2; exit 2 ;;
esac

expected_name="overcrow-$version-x86_64-linux.tar.zst"
if test "${output##*/}" != "$expected_name"; then
    printf '%s\n' 'error: OUTPUT filename does not match VERSION' >&2
    exit 2
fi
if ! test -d "$stage" || ! test -d "$stage/usr"; then
    printf '%s\n' 'error: STAGE must contain a usr directory' >&2
    exit 2
fi

output_parent=$(dirname -- "$output")
if ! test -d "$output_parent"; then
    printf '%s\n' 'error: OUTPUT parent does not exist' >&2
    exit 2
fi
if test -e "$output" || test -L "$output"; then
    printf '%s\n' 'error: OUTPUT already exists' >&2
    exit 2
fi

case ${SOURCE_DATE_EPOCH:-} in
    ''|*[!0-9]*)
        printf '%s\n' \
            'error: SOURCE_DATE_EPOCH must be a non-negative integer' >&2
        exit 2
        ;;
esac

stage_usr=$(CDPATH='' cd -- "$stage/usr" && pwd -P)
output_parent_physical=$(CDPATH='' cd -- "$output_parent" && pwd -P)
case "$output_parent_physical/" in
    "$stage_usr/"*)
        printf '%s\n' 'error: OUTPUT parent must be outside STAGE/usr' >&2
        exit 2
        ;;
esac

working=
cleanup_working() {
    if test -n "$working" &&
            { test -e "$working" || test -L "$working"; }; then
        rm -rf -- "$working"
    fi
}
handle_exit() {
    exit_status=$?
    trap - EXIT HUP INT TERM
    cleanup_working || exit_status=1
    exit "$exit_status"
}
handle_signal() {
    trap - EXIT HUP INT TERM
    cleanup_working || :
    exit 1
}
trap handle_exit EXIT
trap handle_signal HUP INT TERM

working=$(mktemp -d "$output_parent/.overcrow-archive.XXXXXX")
chmod 0700 "$working"
tar_payload="$working/payload.tar"
compressed_payload="$working/bundle.tar.zst"

if ! LC_ALL=C tar \
        --directory "$stage" \
        --sort=name \
        --owner=0 \
        --group=0 \
        --numeric-owner \
        --mtime="@$SOURCE_DATE_EPOCH" \
        --format=posix \
        --pax-option=delete=atime,delete=ctime \
        --file "$tar_payload" \
        --create \
        usr; then
    printf '%s\n' 'error: could not create archive payload' >&2
    exit 1
fi

if ! zstd --quiet -19 --threads=1 \
        -o "$compressed_payload" "$tar_payload"; then
    printf '%s\n' 'error: could not compress archive payload' >&2
    exit 1
fi
if ! test -s "$compressed_payload"; then
    printf '%s\n' 'error: compressed archive payload is empty' >&2
    exit 1
fi
chmod 0644 "$compressed_payload"
rm -f -- "$tar_payload"

if test -e "$output" || test -L "$output"; then
    printf '%s\n' 'error: OUTPUT appeared before publication' >&2
    exit 1
fi
if ! mv -T -n -- "$compressed_payload" "$output"; then
    printf '%s\n' 'error: could not publish OUTPUT' >&2
    exit 1
fi
if test -e "$compressed_payload" || test -L "$compressed_payload"; then
    printf '%s\n' 'error: OUTPUT appeared during publication' >&2
    exit 1
fi
if ! test -f "$output"; then
    printf '%s\n' 'error: publication did not create OUTPUT' >&2
    exit 1
fi

rmdir -- "$working"
working=
