#!/bin/sh
set -eu

usage() {
    printf '%s\n' \
        'usage: render-control.sh VERSION DEPENDENCIES INSTALLED_SIZE OUTPUT' >&2
    exit 2
}

test "$#" -eq 4 || usage
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"

version=$1
dependencies=$2
installed_size=$3
output=$4
template="$project_root/packaging/deb/control.in"

if ! deb_version=$(overcrow_deb_package_version "$version"); then
    printf '%s\n' 'error: invalid package version' >&2
    exit 2
fi
case $dependencies in
    ''|*'
'*)
        printf '%s\n' 'error: dependencies must be one non-empty line' >&2
        exit 2
        ;;
esac
if test "${#dependencies}" -gt 4096 ||
        ! printf '%s\n' "$dependencies" |
            LC_ALL=C grep -Eq '^[A-Za-z0-9][A-Za-z0-9+.:~(),|<>= -]*$'; then
    printf '%s\n' 'error: dependencies contain unsupported characters' >&2
    exit 2
fi
case $installed_size in
    ''|*[!0-9]*)
        printf '%s\n' 'error: installed size must be a non-negative integer' >&2
        exit 2
        ;;
esac
case $output in
    /*) ;;
    *)
        printf '%s\n' 'error: output must be an absolute path' >&2
        exit 2
        ;;
esac
output_parent=$(dirname -- "$output")
test -d "$output_parent" || {
    printf '%s\n' 'error: output parent does not exist' >&2
    exit 2
}
if test -e "$output" || test -L "$output"; then
    printf '%s\n' 'error: output already exists' >&2
    exit 2
fi
if ! test -f "$template" || test -L "$template"; then
    printf '%s\n' 'error: control template is unavailable' >&2
    exit 1
fi
for token in @DEB_VERSION@ @INSTALLED_SIZE@ @DEPENDS@; do
    count=$(grep -Fo "$token" "$template" | wc -l)
    test "$count" -eq 1 || {
        printf '%s\n' "error: invalid control template token: $token" >&2
        exit 1
    }
done

working=
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    if test -n "$working"; then
        rm -f -- "$working"
    fi
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

working=$(mktemp "$output_parent/.overcrow-control.XXXXXX")
chmod 0600 "$working"
awk \
    -v version="$deb_version" \
    -v size="$installed_size" \
    -v dependencies="$dependencies" '
{
    gsub("@DEB_VERSION@", version)
    gsub("@INSTALLED_SIZE@", size)
    gsub("@DEPENDS@", dependencies)
    print
}
' "$template" > "$working"

if grep -Eq '@[A-Z_]+@' "$working"; then
    printf '%s\n' 'error: rendered control contains an unresolved token' >&2
    exit 1
fi
chmod 0644 "$working"
if ! mv -T -n -- "$working" "$output"; then
    printf '%s\n' 'error: could not publish control metadata' >&2
    exit 1
fi
working=
