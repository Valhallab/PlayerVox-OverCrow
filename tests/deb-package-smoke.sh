#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"

test "$(overcrow_deb_upstream_version 0.1.0-pre-alpha.4)" = \
    '0.1.0~pre.alpha.4'
test "$(overcrow_deb_package_version 0.1.0-pre-alpha.4)" = \
    '0.1.0~pre.alpha.4-1'
test "$(overcrow_deb_package_version 1.2.3)" = '1.2.3-1'
if overcrow_deb_package_version '01.2.3' >/dev/null 2>&1; then
    printf '%s\n' 'invalid Cargo version produced a Debian version' >&2
    exit 1
fi

renderer="$project_root/packaging/deb/render-control.sh"
test -x "$renderer"
test -f "$project_root/packaging/deb/control.in"
test -f "$project_root/packaging/deb/copyright"
for extension_file in extension.js metadata.json; do
    grep -Fqx \
        "usr/share/gnome-shell/extensions/overcrow@playervox.com/$extension_file" \
        "$project_root/packaging/release/manifest.txt"
done

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-deb-smoke.XXXXXX")
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    rm -rf -- "$work_dir"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

control="$work_dir/control"
dependencies='libc6 (>= 2.38), libgtk-3-0, systemd, xdg-desktop-portal, xdg-utils'
"$renderer" 0.1.0-pre-alpha.4 "$dependencies" 4242 "$control"

test "$(stat -c '%a' "$control")" = 644
for expected in \
        'Package: overcrow' \
        'Version: 0.1.0~pre.alpha.4-1' \
        'Section: games' \
        'Priority: optional' \
        'Architecture: amd64' \
        'Maintainer: Valhallab <contact@valhallab.com>' \
        'Installed-Size: 4242' \
        "Depends: $dependencies" \
        'Suggests: gnome-shell' \
        'Homepage: https://overcrow.playervox.com' \
        'Description: Opt-in external Linux game overlay by PlayerVox'; do
    test "$(grep -Fxc "$expected" "$control")" -eq 1
done
if grep -Eq '@[A-Z_]+@' "$control"; then
    printf '%s\n' 'rendered Debian control file contains a token' >&2
    exit 1
fi

for invalid_case in \
        'invalid-version' \
        'invalid-dependencies' \
        'invalid-size' \
        'relative-output' \
        'existing-output' \
        'symlink-output'; do
    case $invalid_case in
        invalid-version)
            version='01.2.3'
            candidate_dependencies=$dependencies
            installed_size=4242
            output="$work_dir/invalid-version"
            ;;
        invalid-dependencies)
            version='0.1.0'
            candidate_dependencies=$(printf 'libc6\nMaintainer: attacker')
            installed_size=4242
            output="$work_dir/invalid-dependencies"
            ;;
        invalid-size)
            version='0.1.0'
            candidate_dependencies=$dependencies
            installed_size='-1'
            output="$work_dir/invalid-size"
            ;;
        relative-output)
            version='0.1.0'
            candidate_dependencies=$dependencies
            installed_size=4242
            output='relative-control'
            ;;
        existing-output)
            version='0.1.0'
            candidate_dependencies=$dependencies
            installed_size=4242
            output="$work_dir/existing-output"
            : > "$output"
            ;;
        symlink-output)
            version='0.1.0'
            candidate_dependencies=$dependencies
            installed_size=4242
            output="$work_dir/symlink-output"
            ln -s "$work_dir/missing-target" "$output"
            ;;
    esac
    if "$renderer" "$version" "$candidate_dependencies" "$installed_size" \
            "$output" >/dev/null 2>&1; then
        printf '%s\n' "renderer accepted case: $invalid_case" >&2
        exit 1
    fi
done

grep -Fq 'Valhallab' "$project_root/packaging/deb/copyright"
grep -Fq 'AGPL-3.0-only' "$project_root/packaging/deb/copyright"

builder="$project_root/scripts/build-deb-package.sh"
test -x "$builder"
payload_move_line=$(
    awk 'index($0, "mv \"$stage/usr\" \"$package_root/usr\"") {
        print NR
        exit
    }' "$builder"
)
documentation_dir_line=$(
    awk 'index($0, "\"$package_root/usr/share/doc/overcrow\"") {
        print NR
        exit
    }' "$builder"
)
if test -z "$payload_move_line" || test -z "$documentation_dir_line" ||
        test "$payload_move_line" -ge "$documentation_dir_line"; then
    printf '%s\n' \
        'DEB builder creates the documentation directory before moving usr' >&2
    exit 1
fi
for required_text in \
        'npm ci --ignore-scripts --no-audit --no-fund' \
        'cargo fetch --locked' \
        'cargo build --workspace --release --locked' \
        'packaging/release/stage.sh' \
        'packaging/release/manifest.txt' \
        'dpkg-shlibdeps' \
        'dpkg-deb --root-owner-group' \
        'usr/bin/overcrow-control' \
        'usr/bin/overcrow-core' \
        'usr/bin/overcrow-hyprland' \
        'usr/bin/overcrow-overlay' \
        'usr/bin/overcrowctl' \
        'dbus-user-session' \
        'libayatana-appindicator3-1' \
        'libegl1' \
        'libgl1' \
        'libwayland-client0' \
        'libwayland-egl1' \
        'libx11-6' \
        'libx11-xcb1' \
        'libxcb1' \
        'libxcursor1' \
        'libxi6' \
        'libxkbcommon0' \
        'libxkbcommon-x11-0' \
        'systemd' \
        'xdg-desktop-portal' \
        'xdg-utils' \
        'SOURCE_DATE_EPOCH' \
        'Ubuntu 24.04'; do
    grep -Fq "$required_text" "$builder"
done
if grep -Eq \
        'sudo|apt-get|dpkg[[:space:]]+-i|systemctl|kpackagetool|qdbus|hyprctl|--ignore-missing-info' \
        "$builder"; then
    printf '%s\n' 'DEB builder contains installation or unsafe fallback commands' >&2
    exit 1
fi

printf '%s\n' 'DEB package smoke test passed'
