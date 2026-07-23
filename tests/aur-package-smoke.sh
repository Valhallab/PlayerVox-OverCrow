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
grep -Fqx 'pkgver=0.1.0prealpha1' "$pkgbuild"
grep -Fqx 'pkgrel=1' "$pkgbuild"
grep -Fqx '_release=v0.1.0-pre-alpha.1' "$pkgbuild"
grep -Fqx "arch=('x86_64')" "$pkgbuild"
grep -Fqx "license=('AGPL-3.0-only')" "$pkgbuild"
grep -Fqx "provides=('overcrow')" "$pkgbuild"
grep -Fqx "conflicts=('overcrow')" "$pkgbuild"
grep -Fq "'libayatana-appindicator'" "$pkgbuild"
grep -Fqx 'install=overcrow.install' "$pkgbuild"
grep -Fqx "url='https://github.com/Valhallab/PlayerVox-OverCrow'" "$pkgbuild"
grep -Fq "\${url}/releases/download/\${_release}/" "$pkgbuild"
grep -Fq \
    'c362f154738036290bebeb0740582969cc0ce0b7c659b7cfd8f400695cb5dcd0' \
    "$pkgbuild"
grep -Fq "bsdtar -xf \"\$srcdir/\$_source\"" "$pkgbuild"

grep -Fqx 'pkgbase = overcrow-bin' "$srcinfo"
grep -Fq 'pkgver = 0.1.0prealpha1' "$srcinfo"
grep -Fq 'pkgrel = 1' "$srcinfo"
grep -Fqx '	depends = libayatana-appindicator' "$srcinfo"
grep -Fq \
    'github.com/Valhallab/PlayerVox-OverCrow/releases/download/v0.1.0-pre-alpha.1/' \
    "$srcinfo"
grep -Fq \
    'c362f154738036290bebeb0740582969cc0ce0b7c659b7cfd8f400695cb5dcd0' \
    "$srcinfo"

cmp "$project_root/packaging/arch/overcrow.install" "$install_hook"

if grep -Eq \
        '(^|[;&|[:space:]])(curl|wget|sudo|pacman|systemctl|hyprctl|kwriteconfig|kpackagetool)([;&|[:space:]]|$)' \
        "$pkgbuild" "$install_hook"; then
    printf '%s\n' 'AUR package contains network, privilege, or session mutation commands' >&2
    exit 1
fi

printf '%s\n' 'AUR package smoke test passed'
