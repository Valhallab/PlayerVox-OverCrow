#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)

for removed in \
        crates/overcrow-control-host \
        crates/overcrow-control-ui/src/components/CompanionSetup.tsx \
        packaging/arch/overcrow-companion.install \
        packaging/dbus/io.github.overcrow.ControlHost1.service \
        packaging/flatpak \
        scripts/build-flatpak-bundle.sh \
        tests/flatpak-bundle-smoke.sh \
        tests/flatpak-manifest-smoke.sh; do
    if test -e "$project_root/$removed" || test -L "$project_root/$removed"; then
        printf '%s\n' "obsolete distribution path remains: $removed" >&2
        exit 1
    fi
done

grep -Fqx 'pkgname=overcrow-bin' "$project_root/packaging/arch/PKGBUILD.in"
grep -Fqx 'provides=('"'overcrow'"')' "$project_root/packaging/arch/PKGBUILD.in"

for native_file in \
        Cargo.toml \
        crates/overcrow-control-ui/src-tauri/Cargo.toml \
        crates/overcrow-control-ui/src-tauri/src/commands.rs \
        crates/overcrow-control/src/commands.rs \
        crates/overcrow-control/src/integration.rs \
        crates/overcrow-control/src/lib.rs \
        packaging/release/manifest.txt \
        packaging/release/stage.sh \
        packaging/release/assemble.sh \
        packaging/release/inspect.sh \
        scripts/build-arch-package.sh \
        scripts/prepare-release.sh; do
    if grep -Eiq \
            'overcrow-control-host|ControlHost1|flatpak-control|overcrow-companion|build-flatpak|org\.freedesktop\.Flatpak|ostree' \
            "$project_root/$native_file"; then
        printf '%s\n' "obsolete Flatpak boundary remains in $native_file" >&2
        exit 1
    fi
done

printf '%s\n' 'Native-only distribution smoke test passed'
