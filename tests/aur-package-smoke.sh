#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
package_dir="$project_root/packaging/aur"
pkgbuild="$package_dir/PKGBUILD"
srcinfo="$package_dir/.SRCINFO"
install_hook="$package_dir/overcrow.install"

test -f "$pkgbuild"
test -f "$srcinfo"
test -f "$install_hook"

grep -Fqx '# Maintainer: Valhallab <contact at valhallab dot com>' "$pkgbuild"
grep -Fqx 'pkgname=overcrow-bin' "$pkgbuild"
grep -Fqx 'pkgver=0.1.0prealpha5' "$pkgbuild"
grep -Fqx 'pkgrel=1' "$pkgbuild"
grep -Fqx '_release=v0.1.0-pre-alpha.5' "$pkgbuild"
grep -Fqx "arch=('x86_64')" "$pkgbuild"
grep -Fqx "license=('AGPL-3.0-only')" "$pkgbuild"
grep -Fqx "provides=('overcrow')" "$pkgbuild"
grep -Fqx "conflicts=('overcrow')" "$pkgbuild"
grep -Fq "'libayatana-appindicator'" "$pkgbuild"
grep -Fq "'xdg-utils'" "$pkgbuild"
grep -Fqx 'install=overcrow.install' "$pkgbuild"
grep -Fqx "url='https://github.com/Valhallab/PlayerVox-OverCrow'" "$pkgbuild"
grep -Fq "\${url}/releases/download/\${_release}/" "$pkgbuild"
grep -Fq \
    'f39133b67356b09a3a439dc50eeedf5b887a9b419f5871986308402a8b85b1a7' \
    "$pkgbuild"
grep -Fq "bsdtar -xf \"\$srcdir/\$_source\"" "$pkgbuild"

grep -Fqx 'pkgbase = overcrow-bin' "$srcinfo"
grep -Fq 'pkgver = 0.1.0prealpha5' "$srcinfo"
grep -Fq 'pkgrel = 1' "$srcinfo"
grep -Fqx '	depends = libayatana-appindicator' "$srcinfo"
grep -Fqx '	depends = xdg-utils' "$srcinfo"
grep -Fqx '	optdepends = gnome-shell: GNOME Wayland overlay integration' "$srcinfo"
grep -Fq \
    'github.com/Valhallab/PlayerVox-OverCrow/releases/download/v0.1.0-pre-alpha.5/' \
    "$srcinfo"
grep -Fq \
    'f39133b67356b09a3a439dc50eeedf5b887a9b419f5871986308402a8b85b1a7' \
    "$srcinfo"

cmp "$project_root/packaging/arch/overcrow.install" "$install_hook"

if grep -Eq \
        '(^|[;&|[:space:]])(curl|wget|sudo|pacman|systemctl|hyprctl|kwriteconfig|kpackagetool)([;&|[:space:]]|$)' \
        "$pkgbuild" "$install_hook"; then
    printf '%s\n' 'AUR package contains network, privilege, or session mutation commands' >&2
    exit 1
fi

printf '%s\n' 'AUR package smoke test passed'
