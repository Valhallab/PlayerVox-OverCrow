#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'usage: render-spec.sh VERSION BUNDLE OUTPUT' >&2
    exit 2
}

test "$#" -eq 3 || usage
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
# The source path is derived from this checked-in script's physical root.
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"
template="$project_root/packaging/rpm/overcrow.spec.in"
version=$1
bundle=$2
output=$3

if ! rpm_version=$(overcrow_rpm_version "$version"); then
    printf '%s\n' 'error: invalid version' >&2
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

bundle_name=${bundle##*/}
expected_name="overcrow-$version-x86_64-linux.tar.zst"
if test "$bundle_name" != "$expected_name"; then
    printf '%s\n' 'error: BUNDLE filename does not match VERSION' >&2
    exit 2
fi
if ! test -f "$bundle" || test -L "$bundle"; then
    printf '%s\n' 'error: BUNDLE must be a direct regular file' >&2
    exit 2
fi
if ! test -f "$template" || test -L "$template"; then
    printf '%s\n' 'error: RPM spec template is missing or unsafe' >&2
    exit 1
fi

output_parent=$(dirname -- "$output")
if ! test -d "$output_parent" || test -L "$output_parent"; then
    printf '%s\n' 'error: OUTPUT parent must be a direct directory' >&2
    exit 2
fi
checksum_name="$bundle_name.sha256"
checksum_path="$(dirname -- "$bundle")/$checksum_name"
for target in "$output" "$checksum_path"; do
    if test -e "$target" || test -L "$target"; then
        printf '%s\n' 'error: output already exists' >&2
        exit 2
    fi
done

if LC_ALL=C awk '
    {
        rpm_version_count += gsub(/@RPM_VERSION@/, "&")
        bundle_count += gsub(/@BUNDLE_NAME@/, "&")
        checksum_count += gsub(/@CHECKSUM_NAME@/, "&")
        line = $0
        gsub(/@RPM_VERSION@|@BUNDLE_NAME@|@CHECKSUM_NAME@/, "", line)
        if (line ~ /@[A-Z][A-Z0-9_]*@/) {
            unknown = 1
        }
    }
    END {
        if (rpm_version_count != 1 || bundle_count != 1 ||
                checksum_count != 1 || unknown) {
            exit 42
        }
    }
' "$template"; then
    :
else
    template_status=$?
    if test "$template_status" -eq 42; then
        printf '%s\n' 'error: RPM spec template tokens are invalid' >&2
    else
        printf '%s\n' 'error: could not validate RPM spec template' >&2
    fi
    exit 1
fi

if ! checksum_report=$(LC_ALL=C sha256sum -- "$bundle"); then
    printf '%s\n' 'error: could not checksum BUNDLE' >&2
    exit 1
fi
checksum=${checksum_report%% *}
case $checksum in
    *[!0-9a-f]*|'') printf '%s\n' 'error: invalid BUNDLE checksum' >&2; exit 1 ;;
esac
if test "${#checksum}" -ne 64; then
    printf '%s\n' 'error: invalid BUNDLE checksum' >&2
    exit 1
fi

working_spec=
working_checksum=
published_checksum=false
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    test -z "$working_spec" || rm -f -- "$working_spec"
    test -z "$working_checksum" || rm -f -- "$working_checksum"
    if test "$published_checksum" = true &&
            { ! test -e "$output" || test -L "$output"; }; then
        rm -f -- "$checksum_path"
    fi
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

working_spec=$(mktemp "$output_parent/.overcrow-rpm-spec.XXXXXX")
working_checksum=$(mktemp "$(dirname -- "$bundle")/.overcrow-rpm-checksum.XXXXXX")
chmod 0600 "$working_spec" "$working_checksum"

sed \
    -e "s|@RPM_VERSION@|$rpm_version|g" \
    -e "s|@BUNDLE_NAME@|$bundle_name|g" \
    -e "s|@CHECKSUM_NAME@|$checksum_name|g" \
    "$template" > "$working_spec"
printf '%s  %s\n' "$checksum" "$bundle_name" > "$working_checksum"

if LC_ALL=C grep -Eq '@[A-Z][A-Z0-9_]*@' "$working_spec"; then
    printf '%s\n' 'error: unresolved RPM spec token' >&2
    exit 1
else
    grep_status=$?
    if test "$grep_status" -ne 1; then
        printf '%s\n' 'error: could not scan rendered RPM spec' >&2
        exit 1
    fi
fi

chmod 0644 "$working_spec" "$working_checksum"
mv -T -n -- "$working_checksum" "$checksum_path" ||
    { printf '%s\n' 'error: could not publish checksum' >&2; exit 1; }
working_checksum=
published_checksum=true
mv -T -n -- "$working_spec" "$output" ||
    { printf '%s\n' 'error: could not publish RPM spec' >&2; exit 1; }
working_spec=

if ! test -f "$output" || test -L "$output"; then
    printf '%s\n' 'error: RPM spec publication failed' >&2
    exit 1
fi
published_checksum=false
