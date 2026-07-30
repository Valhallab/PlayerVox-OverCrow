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
grep -Fqx 'pkgver=0.1.0prealpha4' "$pkgbuild"
grep -Fqx 'pkgrel=1' "$pkgbuild"
grep -Fqx '_release=v0.1.0-pre-alpha.4' "$pkgbuild"
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
    'dd4525a88e50b52c8c4e31aab6b7610736ecc70b8cb76ecc70102878161a711b' \
    "$pkgbuild"
grep -Fq "bsdtar -xf \"\$srcdir/\$_source\"" "$pkgbuild"

grep -Fqx 'pkgbase = overcrow-bin' "$srcinfo"
grep -Fq 'pkgver = 0.1.0prealpha4' "$srcinfo"
grep -Fq 'pkgrel = 1' "$srcinfo"
grep -Fqx '	depends = libayatana-appindicator' "$srcinfo"
grep -Fqx '	depends = xdg-utils' "$srcinfo"
grep -Fq \
    'github.com/Valhallab/PlayerVox-OverCrow/releases/download/v0.1.0-pre-alpha.4/' \
    "$srcinfo"
grep -Fq \
    'dd4525a88e50b52c8c4e31aab6b7610736ecc70b8cb76ecc70102878161a711b' \
    "$srcinfo"

cmp "$project_root/packaging/arch/overcrow.install" "$install_hook"

if grep -Eq \
        '(^|[;&|[:space:]])(curl|wget|sudo|pacman|systemctl|hyprctl|kwriteconfig|kpackagetool)([;&|[:space:]]|$)' \
        "$pkgbuild" "$install_hook"; then
    printf '%s\n' 'AUR package contains network, privilege, or session mutation commands' >&2
    exit 1
fi

printf '%s\n' 'AUR package smoke test passed'
