#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'usage: prepare-release.sh' >&2
    exit 2
}

test "$#" -eq 0 || usage
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
# The source path is derived from this checked-in script's physical root.
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"

cd "$project_root"
branch=$(git symbolic-ref --quiet --short HEAD) || {
    printf '%s\n' 'error: release preparation requires branch master' >&2
    exit 1
}
if test "$branch" != master; then
    printf '%s\n' 'error: release preparation requires branch master' >&2
    exit 1
fi
if test -n "$(git status --porcelain --untracked-files=all)"; then
    printf '%s\n' 'error: release preparation requires a clean worktree' >&2
    exit 1
fi

for program in bsdtar cargo git node npm sha256sum shellcheck; do
    command -v "$program" >/dev/null 2>&1 || {
        printf '%s\n' "error: required release tool is unavailable: $program" >&2
        exit 1
    }
done
if ! cargo deny --version >/dev/null 2>&1; then
    printf '%s\n' 'error: required Cargo subcommand is unavailable: cargo deny' >&2
    exit 1
fi

package_id=$(cargo pkgid -p overcrow-control)
version=${package_id##*#}
if ! overcrow_version_is_valid "$version"; then
    printf '%s\n' "error: invalid workspace version: $version" >&2
    exit 1
fi
if ! arch_version=$(overcrow_arch_version "$version"); then
    printf '%s\n' "error: cannot normalize Arch version: $version" >&2
    exit 1
fi
if test "$(uname -m)" != x86_64; then
    printf '%s\n' 'error: releases are currently built for x86_64 only' >&2
    exit 1
fi

SOURCE_DATE_EPOCH=$(git show -s --format=%ct HEAD)
case $SOURCE_DATE_EPOCH in
    ''|*[!0-9]*)
        printf '%s\n' 'error: HEAD has no valid commit timestamp' >&2
        exit 1
        ;;
esac
export SOURCE_DATE_EPOCH

for output in \
        "$project_root/dist/overcrow-bin-$arch_version-1-x86_64.pkg.tar.zst" \
        "$project_root/dist/release"; do
    if test -e "$output" || test -L "$output"; then
        printf '%s\n' "error: remove previous release output first: $output" >&2
        exit 1
    fi
done

npm --prefix "$project_root/crates/overcrow-control-ui" ci \
    --ignore-scripts --no-audit --no-fund
npm --prefix "$project_root/crates/overcrow-control-ui" test
npm --prefix "$project_root/crates/overcrow-control-ui" run build

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --locked
cargo deny --locked check advisories licenses
cargo deny --locked check bans sources

shellcheck scripts/*.sh scripts/lib/*.sh tests/*.sh \
    packaging/arch/*.install packaging/arch/*.sh packaging/aur/*.install \
    packaging/release/*.sh
shellcheck -s bash packaging/aur/PKGBUILD
sh -n scripts/*.sh scripts/lib/*.sh tests/*.sh \
    packaging/arch/*.install packaging/arch/*.sh packaging/aur/*.install \
    packaging/release/*.sh
bash -n packaging/aur/PKGBUILD
node --check integrations/kwin/contents/code/main.js
node --test tests/kwin-bridge.test.js

for smoke_test in tests/*-smoke.sh; do
    "$smoke_test"
done

"$project_root/scripts/build-arch-package.sh"
"$project_root/packaging/release/inspect.sh" \
    "$version" "$project_root/dist"
"$project_root/packaging/release/assemble.sh" \
    "$version" "$project_root/dist" "$project_root/dist/release"
