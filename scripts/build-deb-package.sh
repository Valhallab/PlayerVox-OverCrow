#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
# The source path is derived from this checked-in script's physical root.
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"
dist_dir="$project_root/dist"

package_id=$(cd "$project_root" && cargo pkgid -p overcrow-control)
version=${package_id##*#}
if ! overcrow_version_is_valid "$version"; then
    printf '%s\n' "error: invalid workspace version: $version" >&2
    exit 1
fi
if ! deb_version=$(overcrow_deb_package_version "$version"); then
    printf '%s\n' "error: cannot normalize Debian version: $version" >&2
    exit 1
fi

if test "$(id -u)" -eq 0; then
    printf '%s\n' 'error: build the package as a regular desktop user' >&2
    exit 1
fi
if test "$(uname -m)" != x86_64; then
    printf '%s\n' 'error: the initial DEB target requires x86_64' >&2
    exit 1
fi

os_release_value() {
    key=$1
    awk -v key="$key" '
        index($0, "=") {
            candidate = substr($0, 1, index($0, "=") - 1)
            if (candidate != key) {
                next
            }
            count += 1
            value = substr($0, index($0, "=") + 1)
            if (value ~ /^"[^"]*"$/) {
                value = substr(value, 2, length(value) - 2)
            }
            result = value
        }
        END {
            if (count != 1 || result == "") {
                exit 1
            }
            print result
        }
    ' /etc/os-release
}

host_id=$(os_release_value ID) || {
    printf '%s\n' 'error: could not identify the build distribution' >&2
    exit 1
}
host_version=$(os_release_value VERSION_ID) || {
    printf '%s\n' 'error: could not identify the build distribution version' >&2
    exit 1
}
if test "$host_id" != ubuntu || test "$host_version" != 24.04; then
    printf '%s\n' \
        "error: this compatibility baseline requires Ubuntu 24.04, got: $host_id $host_version" >&2
    exit 1
fi

for program in awk cargo cmp diff dpkg-deb dpkg-shlibdeps du find grep \
        install md5sum node npm sort tar touch; do
    command -v "$program" >/dev/null 2>&1 || {
        printf '%s\n' "error: required build tool is unavailable: $program" >&2
        exit 1
    }
done

mkdir -p "$dist_dir"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-deb-package.XXXXXX")
chmod 0700 "$work_dir"
published_work=
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    if test -n "$published_work"; then
        rm -f -- "$published_work"
    fi
    rm -rf -- "$work_dir"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

publish_artifact() {
    source_path=$1
    artifact=$2
    published_work=$(mktemp "$dist_dir/.overcrow-deb-package.XXXXXX")
    install -m 0644 "$source_path" "$published_work"
    mv -T -f -- "$published_work" "$artifact"
    published_work=
    printf '\n%s\n' "DEB package ready: $artifact"
}

: "${SOURCE_DATE_EPOCH:=$(date +%s)}"
case $SOURCE_DATE_EPOCH in
    ''|*[!0-9]*)
        printf '%s\n' 'error: SOURCE_DATE_EPOCH must be a non-negative integer' >&2
        exit 1
        ;;
esac
export SOURCE_DATE_EPOCH

printf '%s\n' "Building PlayerVox OverCrow $version for Ubuntu 24.04..."
(
    cd "$project_root/crates/overcrow-control-ui"
    npm ci --ignore-scripts --no-audit --no-fund
    npm run build
)
remap_flag="--remap-path-prefix=$project_root=/usr/src/overcrow"
if test -n "${RUSTFLAGS:-}"; then
    RUSTFLAGS="$RUSTFLAGS $remap_flag"
else
    RUSTFLAGS=$remap_flag
fi
export RUSTFLAGS
cd "$project_root"
cargo fetch --locked
cargo build --workspace --release --locked

notices="$work_dir/THIRD_PARTY_LICENSES.md"
"$project_root/scripts/generate-third-party-notices.sh" "$notices"

stage="$work_dir/stage"
"$project_root/packaging/release/stage.sh" "$stage" "$notices"
(
    cd "$stage"
    find usr -type f -print | LC_ALL=C sort > "$work_dir/shared-manifest.txt"
)
if ! cmp -s "$project_root/packaging/release/manifest.txt" \
        "$work_dir/shared-manifest.txt"; then
    printf '%s\n' 'error: staged package does not match the release manifest' >&2
    diff -u "$project_root/packaging/release/manifest.txt" \
        "$work_dir/shared-manifest.txt" >&2 || true
    exit 1
fi

package_root="$work_dir/package"
install -d -m 0755 "$package_root/DEBIAN"
mv "$stage/usr" "$package_root/usr"
install -d -m 0755 "$package_root/usr/share/doc/overcrow"
install -m 0644 "$project_root/packaging/deb/copyright" \
    "$package_root/usr/share/doc/overcrow/copyright"

{
    cat "$project_root/packaging/release/manifest.txt"
    printf '%s\n' 'usr/share/doc/overcrow/copyright'
} | LC_ALL=C sort > "$work_dir/expected-deb-manifest.txt"
(
    cd "$package_root"
    find usr -type f -print | LC_ALL=C sort > "$work_dir/deb-manifest.txt"
)
if ! cmp -s "$work_dir/expected-deb-manifest.txt" \
        "$work_dir/deb-manifest.txt"; then
    printf '%s\n' 'error: Debian payload does not match its approved manifest' >&2
    diff -u "$work_dir/expected-deb-manifest.txt" \
        "$work_dir/deb-manifest.txt" >&2 || true
    exit 1
fi

shlibdeps_root="$work_dir/shlibdeps"
install -d -m 0755 "$shlibdeps_root/debian"
cat > "$shlibdeps_root/debian/control" <<'EOF'
Source: overcrow
Section: games
Priority: optional
Maintainer: Valhallab <contact@valhallab.com>
Standards-Version: 4.7.4
Rules-Requires-Root: no

Package: overcrow
Architecture: amd64
Depends: ${shlibs:Depends}
Description: Opt-in external Linux game overlay by PlayerVox
 PlayerVox OverCrow provides movable widgets without game-process injection.
EOF
shlibdeps_output=$(
    cd "$shlibdeps_root"
    dpkg-shlibdeps -O \
        -e"$package_root/usr/bin/overcrow-control" \
        -e"$package_root/usr/bin/overcrow-core" \
        -e"$package_root/usr/bin/overcrow-hyprland" \
        -e"$package_root/usr/bin/overcrow-overlay" \
        -e"$package_root/usr/bin/overcrowctl"
)
if test "$(printf '%s\n' "$shlibdeps_output" |
        grep -c '^shlibs:Depends=')" -ne 1; then
    printf '%s\n' 'error: shared-library dependency output is invalid' >&2
    exit 1
fi
shlibdeps=${shlibdeps_output#shlibs:Depends=}
case $shlibdeps in
    ''|*'
'*)
        printf '%s\n' 'error: shared-library dependency output is invalid' >&2
        exit 1
        ;;
esac

# These libraries are loaded at runtime by the tray and display backends, so
# ELF dependency inspection cannot discover them.
runtime_dependencies='
dbus-user-session
libayatana-appindicator3-1
libegl1
libgl1
libwayland-client0
libwayland-egl1
libx11-6
libx11-xcb1
libxcb1
libxcursor1
libxi6
libxkbcommon0
libxkbcommon-x11-0
systemd
xdg-desktop-portal
xdg-utils
'
dependencies=$shlibdeps
for requirement in $runtime_dependencies; do
    case ", $dependencies," in
        *", $requirement "*|*", $requirement,")
            ;;
        *)
            dependencies="$dependencies, $requirement"
            ;;
    esac
done
installed_size=$(du -sk "$package_root/usr" | awk '{ print $1 }')
"$project_root/packaging/deb/render-control.sh" \
    "$version" "$dependencies" "$installed_size" "$package_root/DEBIAN/control"

(
    cd "$package_root"
    find usr -type f -print | LC_ALL=C sort |
        while IFS= read -r file; do
            md5sum "$file"
        done
) > "$package_root/DEBIAN/md5sums"
chmod 0644 "$package_root/DEBIAN/md5sums"

find "$package_root" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
built_deb="$work_dir/overcrow_${deb_version}_amd64.deb"
dpkg-deb --root-owner-group -Zxz -z9 --build "$package_root" "$built_deb"
if ! test -f "$built_deb" || test -L "$built_deb" || ! test -s "$built_deb"; then
    printf '%s\n' 'error: dpkg-deb did not produce the expected package' >&2
    exit 1
fi
set -- "$work_dir"/*.deb
if test "$#" -ne 1 || test "$1" != "$built_deb"; then
    printf '%s\n' 'error: dpkg-deb produced an unexpected package set' >&2
    exit 1
fi

# dpkg-deb expands its own field placeholders; the shell must not.
# shellcheck disable=SC2016
identity=$(dpkg-deb --show \
    --showformat='${Package}|${Version}|${Architecture}\n' "$built_deb")
expected_identity="overcrow|$deb_version|amd64"
if test "$identity" != "$expected_identity"; then
    printf '%s\n' "error: unexpected DEB identity: $identity" >&2
    exit 1
fi
package_dependencies=$(dpkg-deb --field "$built_deb" Depends)
for requirement in $runtime_dependencies; do
    if ! printf '%s\n' "$package_dependencies" |
            grep -Eq "(^|, )$requirement(\$|, )"; then
        printf '%s\n' "error: DEB dependency is missing: $requirement" >&2
        exit 1
    fi
done

dpkg-deb --ctrl-tarfile "$built_deb" | tar -tf - |
    LC_ALL=C sort > "$work_dir/control-files.txt"
printf '%s\n' ./ ./control ./md5sums | LC_ALL=C sort \
    > "$work_dir/expected-control-files.txt"
if ! cmp -s "$work_dir/expected-control-files.txt" \
        "$work_dir/control-files.txt"; then
    printf '%s\n' 'error: DEB contains unexpected control metadata or scripts' >&2
    diff -u "$work_dir/expected-control-files.txt" \
        "$work_dir/control-files.txt" >&2 || true
    exit 1
fi

dpkg-deb --contents "$built_deb" > "$work_dir/package-files.txt"
if ! awk '
    $1 ~ /^l/ ||
    substr($1, 6, 1) == "w" ||
    substr($1, 9, 1) == "w" ||
    $2 != "root/root" {
        exit 42
    }
' "$work_dir/package-files.txt"; then
    printf '%s\n' 'error: DEB contains unsafe ownership, permissions, or symlinks' >&2
    exit 1
fi
awk '
    $1 !~ /^d/ {
        path = $NF
        sub("^\\./", "", path)
        print path
    }
' "$work_dir/package-files.txt" | LC_ALL=C sort \
    > "$work_dir/built-deb-manifest.txt"
if ! cmp -s "$work_dir/expected-deb-manifest.txt" \
        "$work_dir/built-deb-manifest.txt"; then
    printf '%s\n' 'error: built DEB payload does not match the approved manifest' >&2
    diff -u "$work_dir/expected-deb-manifest.txt" \
        "$work_dir/built-deb-manifest.txt" >&2 || true
    exit 1
fi

artifact="$dist_dir/overcrow_${deb_version}_amd64.deb"
publish_artifact "$built_deb" "$artifact"
printf '%s\n' 'Nothing was installed or started.'
