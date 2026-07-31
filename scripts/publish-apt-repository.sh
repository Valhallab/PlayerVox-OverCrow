#!/bin/sh
set -eu

usage() {
    printf '%s\n' \
        'usage: publish-apt-repository.sh VERSION SIGNING_FINGERPRINT [--push]' \
        >&2
    exit 2
}

case $# in
    2) push=false ;;
    3)
        test "$3" = --push || usage
        push=true
        ;;
    *) usage ;;
esac

version=$1
signing_fingerprint=$2
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
builder="$project_root/packaging/apt/build-repository.sh"
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"
branch=gh-pages
timeout_seconds=60

fail() {
    printf '%s\n' "error: $*" >&2
    exit 1
}

test -x "$builder" || fail 'APT repository builder is unavailable'
if ! overcrow_version_is_valid "$version"; then
    fail 'invalid release version'
fi
if ! printf '%s\n' "$signing_fingerprint" |
        LC_ALL=C grep -Eq '^[0-9A-F]{40}$'; then
    fail 'SIGNING_FINGERPRINT must be one full uppercase OpenPGP fingerprint'
fi
for program in cat cp diff find git grep install mktemp mv realpath rm \
        timeout; do
    command -v "$program" >/dev/null 2>&1 ||
        fail "required program is unavailable: $program"
done

case ${OVERCROW_APT_TEST_MODE-} in
    '')
        test -z "${OVERCROW_APT_REMOTE_URL-}" ||
            fail 'OVERCROW_APT_REMOTE_URL is restricted to test mode'
        test -z "${OVERCROW_APT_RELEASE_DIR-}" ||
            fail 'OVERCROW_APT_RELEASE_DIR is restricted to test mode'
        test -z "${OVERCROW_APT_OUTPUT_DIR-}" ||
            fail 'OVERCROW_APT_OUTPUT_DIR is restricted to test mode'
        remote_url='git@github.com:Valhallab/PlayerVox-OverCrow.git'
        release_dir="$project_root/dist/release"
        output_dir="$project_root/dist/apt-repository"
        ;;
    1)
        remote_url=${OVERCROW_APT_REMOTE_URL-}
        release_dir=${OVERCROW_APT_RELEASE_DIR-}
        output_dir=${OVERCROW_APT_OUTPUT_DIR-}
        case $remote_url in
            /*) ;;
            *) fail 'test remote must be an absolute local path' ;;
        esac
        test -d "$remote_url" && test ! -L "$remote_url" ||
            fail 'test remote must be a real directory'
        test "$(realpath -e -- "$remote_url")" = "$remote_url" ||
            fail 'test remote must already be canonical'
        ;;
    *) fail 'OVERCROW_APT_TEST_MODE must be empty or 1' ;;
esac

case $release_dir in
    /*) ;;
    *) fail 'release directory must be absolute' ;;
esac
test -d "$release_dir" && test ! -L "$release_dir" ||
    fail 'release directory is unavailable'
case $output_dir in
    /*) ;;
    *) fail 'output directory must be absolute' ;;
esac
output_parent=$(dirname -- "$output_dir")
if test -e "$output_parent" || test -L "$output_parent"; then
    test -d "$output_parent" && test ! -L "$output_parent" ||
        fail 'output parent is invalid'
else
    install -d -m 0755 "$output_parent"
fi
output_parent=$(realpath -e -- "$output_parent") ||
    fail 'output parent cannot be canonicalized'
test "$output_dir" = "$output_parent/$(basename -- "$output_dir")" ||
    fail 'output directory must already be canonical'

if test -z "${SOURCE_DATE_EPOCH-}"; then
    source_epoch=$(
        git -C "$project_root" show -s --format=%ct \
            "v$version^{commit}" 2>/dev/null
    ) || fail 'release tag is unavailable for SOURCE_DATE_EPOCH'
    case $source_epoch in
        '' | *[!0-9]*) fail 'release tag has an invalid timestamp' ;;
    esac
    SOURCE_DATE_EPOCH=$source_epoch
    export SOURCE_DATE_EPOCH
fi

work_dir=
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    if test -n "$work_dir" &&
            { test -e "$work_dir" || test -L "$work_dir"; }; then
        rm -rf -- "$work_dir"
    fi
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

umask 077
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-apt-publish.XXXXXX")
checkout="$work_dir/checkout"
empty_base="$work_dir/empty-base"
generated="$work_dir/generated"
mkdir "$empty_base"

set +e
GIT_TERMINAL_PROMPT=0 timeout "$timeout_seconds" \
    git ls-remote --exit-code --heads "$remote_url" \
    "refs/heads/$branch" > "$work_dir/remote-head"
remote_status=$?
set -e
case $remote_status in
    0)
        GIT_TERMINAL_PROMPT=0 timeout "$timeout_seconds" \
            git clone --quiet --depth 1 --branch "$branch" \
            "$remote_url" "$checkout" ||
            fail 'cannot clone the current APT repository'
        base_repository=$checkout
        ;;
    2)
        base_repository=$empty_base
        ;;
    124) fail 'APT repository lookup timed out' ;;
    *) fail 'cannot inspect the APT repository remote' ;;
esac

SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH \
    "$builder" "$version" "$release_dir" "$base_repository" \
    "$generated" "$signing_fingerprint"

test -f "$generated/.overcrow-generated-apt-repository" &&
        test ! -L "$generated/.overcrow-generated-apt-repository" &&
        test "$(cat "$generated/.overcrow-generated-apt-repository")" = 1 ||
    fail 'generated repository ownership marker is invalid'
test -z "$(find "$generated" -type l -print -quit)" ||
    fail 'generated repository contains a symlink'

if test -e "$output_dir" || test -L "$output_dir"; then
    test -d "$output_dir" && test ! -L "$output_dir" &&
            test -f "$output_dir/.overcrow-generated-apt-repository" &&
            test ! -L "$output_dir/.overcrow-generated-apt-repository" &&
            test "$(
                cat "$output_dir/.overcrow-generated-apt-repository"
            )" = 1 ||
        fail 'refusing to replace an unmanaged local candidate'
    rm -rf -- "$output_dir"
fi
mv -T -n -- "$generated" "$output_dir" ||
    fail 'cannot publish the local APT repository candidate'

if test "$push" = false; then
    printf '%s\n' "APT repository candidate ready: $output_dir"
    printf '%s\n' 'Nothing was installed, committed, pushed, or started.'
    exit 0
fi

if test "$remote_status" -eq 2; then
    mkdir "$checkout"
    git -C "$checkout" init -b "$branch" >/dev/null ||
        fail 'cannot initialize the APT publication branch'
    git -C "$checkout" remote add origin "$remote_url" ||
        fail 'cannot configure the APT publication remote'
fi

if test "$remote_status" -eq 0 &&
        diff -qr \
            --exclude=.git \
            --exclude=InRelease \
            --exclude=Release.gpg \
            "$checkout" "$output_dir" >/dev/null 2>&1; then
    printf '%s\n' 'APT repository content is already published'
    exit 0
fi

find "$checkout" -mindepth 1 -maxdepth 1 ! -name .git \
    -exec rm -rf -- {} +
cp -a "$output_dir/." "$checkout/"
test -z "$(find "$checkout" -path "$checkout/.git" -prune -o \
    -type l -print -quit)" || fail 'publication checkout contains a symlink'

git -C "$checkout" add -A
git -C "$checkout" diff --cached --check
if git -C "$checkout" diff --cached --quiet; then
    printf '%s\n' 'APT repository content is already published'
    exit 0
fi
git -C "$checkout" \
    -c user.name=Valhallab \
    -c user.email=contact@valhallab.com \
    -c commit.gpgsign=false \
    commit -m "Publish APT repository for $version" >/dev/null ||
    fail 'cannot commit the APT repository publication'
GIT_TERMINAL_PROMPT=0 timeout "$timeout_seconds" \
    git -C "$checkout" push origin "HEAD:refs/heads/$branch" ||
    fail 'cannot push the APT repository publication'

printf '%s\n' "APT repository published from: $output_dir"
printf '%s\n' "Remote branch updated: $branch"
