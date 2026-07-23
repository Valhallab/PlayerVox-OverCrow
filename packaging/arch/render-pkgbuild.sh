#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'usage: render-pkgbuild.sh VERSION BUNDLE OUTPUT' >&2
    exit 2
}

test "$#" -eq 3 || usage
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
# The source path is derived from this checked-in script's physical root.
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"
template="$project_root/packaging/arch/PKGBUILD.in"
version=$1
bundle=$2
output=$3

if ! overcrow_version_is_valid "$version"; then
    printf '%s\n' 'error: invalid version' >&2
    exit 2
fi
if ! arch_version=$(overcrow_arch_version "$version"); then
    printf '%s\n' 'error: could not normalize Arch version' >&2
    exit 2
fi
case $bundle in
    /*) ;;
    *) printf '%s\n' 'error: BUNDLE must be absolute' >&2; exit 2 ;;
esac
case $output in
    /*) ;;
    *) printf '%s\n' 'error: OUTPUT must be absolute' >&2; exit 2 ;;
esac

expected_name="overcrow-$version-x86_64-linux.tar.zst"
if test "${bundle##*/}" != "$expected_name"; then
    printf '%s\n' 'error: BUNDLE filename does not match VERSION' >&2
    exit 2
fi
if ! test -f "$bundle"; then
    printf '%s\n' 'error: BUNDLE is not a regular file' >&2
    exit 2
fi
if ! test -f "$template"; then
    printf '%s\n' 'error: PKGBUILD template is missing' >&2
    exit 1
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

if LC_ALL=C awk '
    {
        version_count += gsub(/@VERSION@/, "&")
        arch_version_count += gsub(/@ARCH_VERSION@/, "&")
        checksum_count += gsub(/@BUNDLE_SHA256@/, "&")
        line = $0
        gsub(/@VERSION@|@ARCH_VERSION@|@BUNDLE_SHA256@/, "", line)
        if (line ~ /@[A-Z][A-Z0-9_]*@/) {
            unknown = 1
        }
    }
    END {
        if (version_count != 1 || arch_version_count != 1 ||
                checksum_count != 1 || unknown) {
            exit 42
        }
    }
' "$template"; then
    :
else
    template_status=$?
    if test "$template_status" -eq 42; then
        printf '%s\n' 'error: PKGBUILD template tokens are invalid' >&2
    else
        printf '%s\n' 'error: could not validate PKGBUILD template' >&2
    fi
    exit 1
fi

if ! checksum_report=$(LC_ALL=C sha256sum -- "$bundle"); then
    printf '%s\n' 'error: could not checksum BUNDLE' >&2
    exit 1
fi
checksum=${checksum_report%% *}
if test "${#checksum}" -ne 64; then
    printf '%s\n' 'error: invalid BUNDLE checksum' >&2
    exit 1
fi
case $checksum in
    *[!0-9a-f]*)
        printf '%s\n' 'error: invalid BUNDLE checksum' >&2
        exit 1
        ;;
esac

working=
cleanup_working() {
    if test -n "$working" &&
            { test -e "$working" || test -L "$working"; }; then
        rm -f -- "$working"
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

working=$(mktemp "$output_parent/.overcrow-pkgbuild.XXXXXX")
chmod 0600 "$working"
if ! sed \
        -e "s|@VERSION@|$version|g" \
        -e "s|@ARCH_VERSION@|$arch_version|g" \
        -e "s|@BUNDLE_SHA256@|$checksum|g" \
        "$template" > "$working"; then
    printf '%s\n' 'error: could not render PKGBUILD' >&2
    exit 1
fi

if token_matches=$(LC_ALL=C awk '
    /@[A-Z][A-Z0-9_]*@/ {
        print FNR ":" $0
        found = 1
    }
    END {
        if (found) {
            exit 42
        }
    }
' "$working"); then
    :
else
    token_status=$?
    if test "$token_status" -eq 42; then
        printf '%s\n' "$token_matches" >&2
        printf '%s\n' 'error: unresolved PKGBUILD token' >&2
    else
        printf '%s\n' 'error: could not scan rendered PKGBUILD' >&2
    fi
    exit 1
fi

chmod 0644 "$working"
if test -e "$output" || test -L "$output"; then
    printf '%s\n' 'error: OUTPUT appeared before publication' >&2
    exit 1
fi
if ! mv -T -n -- "$working" "$output"; then
    printf '%s\n' 'error: could not publish OUTPUT' >&2
    exit 1
fi
if test -e "$working" || test -L "$working"; then
    printf '%s\n' 'error: OUTPUT appeared during publication' >&2
    exit 1
fi
if ! test -f "$output"; then
    printf '%s\n' 'error: publication did not create OUTPUT' >&2
    exit 1
fi
working=
