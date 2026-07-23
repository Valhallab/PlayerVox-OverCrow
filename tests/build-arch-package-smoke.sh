#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
builder="$repo_root/scripts/build-arch-package.sh"

fail() {
    printf '%s\n' "Arch package builder smoke test failed: $1" >&2
    exit 1
}

[ -x "$builder" ] || fail 'scripts/build-arch-package.sh is missing or not executable'
sh -n "$builder"

for contract in \
    'npm ci --ignore-scripts' \
    'npm run build' \
    'cargo build --workspace --release --locked' \
    '--remap-path-prefix=' \
    'scripts/generate-third-party-notices.sh' \
    'packaging/release/stage.sh' \
    'packaging/release/archive.sh' \
    'packaging/arch/render-pkgbuild.sh' \
    'makepkg' \
    'dist'; do
    grep -Fq -- "$contract" "$builder" || fail "missing build step: $contract"
done

grep -Fq 'for program in cargo makepkg node npm zstd readelf' "$builder" || \
    fail 'the builder does not validate its frontend tools'

for binary in \
    overcrow-control overcrow-core overcrow-hyprland overcrow-overlay overcrowctl; do
    grep -Fqx "usr/bin/$binary" "$repo_root/packaging/release/manifest.txt" || \
        fail "release manifest is missing $binary"
done

grep -Fq "options=('!debug' '!strip')" "$repo_root/packaging/arch/PKGBUILD.in" || \
    fail 'PKGBUILD repeats debug extraction or stripping on release binaries'
grep -Fqx 'pkgname=overcrow-bin' \
    "$repo_root/packaging/arch/PKGBUILD.in" || \
    fail 'PKGBUILD does not produce the single native package'
# This is an intentional literal command shape read from the target script.
# shellcheck disable=SC2016
grep -Fq 'publish_artifact "$1" "$artifact"' "$builder" || \
    fail 'the builder does not publish the native package'

if grep -Eiq 'docker|podman|sudo|pacman[[:space:]]+-[US]|systemctl|hyprctl' "$builder"; then
    fail 'the builder performs installation, session mutation, or container work'
fi
