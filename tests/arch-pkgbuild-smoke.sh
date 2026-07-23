#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$project_root"

archive_script="$project_root/packaging/release/archive.sh"
renderer="$project_root/packaging/arch/render-pkgbuild.sh"
template="$project_root/packaging/arch/PKGBUILD.in"

test -x "$archive_script"
test -x "$renderer"
test -f "$template"
test -f packaging/arch/overcrow.install

grep -Fqx 'pkgname=overcrow-bin' "$template"
grep -Fqx "license=('AGPL-3.0-only')" "$template"
grep -Fqx "provides=('overcrow')" "$template"
grep -Fqx "options=('!debug' '!strip')" "$template"
grep -Fq "'gtk3' 'webkit2gtk-4.1'" "$template"
grep -Fq "'libayatana-appindicator'" "$template"
grep -Fqx '_overcrow_version=@VERSION@' "$template"
grep -Fqx 'pkgver=@ARCH_VERSION@' "$template"
test "$(grep -c '^package' "$template")" -eq 1

if grep -Eiq \
        'pkgbase=|package_[a-z]|makedepends|cargo|rust|git\+|github\.com|curl|wget|sudo|pacman|systemctl' \
        "$template"; then
    printf '%s\n' 'PKGBUILD contains split, build, fetch, or installation behavior' >&2
    exit 1
fi
if grep -Eq 'systemctl --user|hyprctl|kwriteconfig|kpackagetool' \
        packaging/arch/overcrow.install; then
    printf '%s\n' 'package install hook mutates a user session' >&2
    exit 1
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-arch-package.XXXXXX")
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    rm -rf -- "$tmpdir"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

failures=0
review_failure() {
    printf '%s\n' "$1" >&2
    failures=$((failures + 1))
}
expect_failure() {
    label=$1
    expected_status=$2
    expected_error=$3
    shift 3

    if "$@" > "$tmpdir/$label.out" 2> "$tmpdir/$label.err"; then
        actual_status=0
    else
        actual_status=$?
    fi
    if test "$actual_status" -ne "$expected_status"; then
        review_failure "$label returned $actual_status instead of $expected_status"
    fi
    if ! grep -Fqx "$expected_error" "$tmpdir/$label.err"; then
        review_failure "$label did not report: $expected_error"
    fi
}

stage1="$tmpdir/stage-one"
stage2="$tmpdir/stage-two"
for stage in "$stage1" "$stage2"; do
    install -d -m 0755 \
        "$stage/usr/bin" \
        "$stage/usr/lib/overcrow" \
        "$stage/usr/share/applications" \
        "$stage/usr/share/licenses/overcrow"
    for binary in overcrow-control overcrow-core overcrow-overlay; do
        printf '%s\n' '#!/bin/sh' 'exit 0' > "$stage/usr/bin/$binary"
        chmod 0755 "$stage/usr/bin/$binary"
    done
    printf '%s\n' '#!/bin/sh' 'exit 0' \
        > "$stage/usr/lib/overcrow/overcrow-integrate"
    chmod 0755 "$stage/usr/lib/overcrow/overcrow-integrate"
    printf '%s\n' '[Desktop Entry]' 'Name=PlayerVox OverCrow' \
        > "$stage/usr/share/applications/com.playervox.OverCrow.desktop"
    printf '%s\n' 'AGPL license sentinel' \
        > "$stage/usr/share/licenses/overcrow/LICENSE"
done

find "$stage1" -exec touch -h -d '@1600000000' {} +
find "$stage2" -exec touch -h -d '@1800000000' {} +
find "$stage1" -type f -exec sh -c 'for path do : < "$path"; done' sh {} +

archive_dir1="$tmpdir/archive-one"
archive_dir2="$tmpdir/archive-two"
install -d -m 0755 "$archive_dir1" "$archive_dir2"
archive1="$archive_dir1/overcrow-0.1.0-x86_64-linux.tar.zst"
archive2="$archive_dir2/overcrow-0.1.0-x86_64-linux.tar.zst"
source_date_epoch=1700000000

expect_failure invalid-archive-version 2 'error: invalid version' \
    env SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" '0.1.0-' "$stage1" "$archive1"
expect_failure relative-archive-stage 2 'error: STAGE must be absolute' \
    env SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" 0.1.0 relative-stage "$archive1"
expect_failure relative-archive-output 2 'error: OUTPUT must be absolute' \
    env SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" 0.1.0 "$stage1" relative-output
expect_failure invalid-archive-name 2 \
    'error: OUTPUT filename does not match VERSION' \
    env SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" 0.1.0 "$stage1" "$archive_dir1/wrong-name.tar.zst"
expect_failure invalid-archive-epoch 2 \
    'error: SOURCE_DATE_EPOCH must be a non-negative integer' \
    env SOURCE_DATE_EPOCH=invalid \
    "$archive_script" 0.1.0 "$stage1" "$archive1"

existing_archive="$archive_dir1/overcrow-1.0.0-x86_64-linux.tar.zst"
printf '%s\n' 'existing archive owner' > "$existing_archive"
expect_failure existing-archive 2 'error: OUTPUT already exists' \
    env SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" 1.0.0 "$stage1" "$existing_archive"
grep -Fqx 'existing archive owner' "$existing_archive" ||
    review_failure 'archive creation replaced an existing output'

producer_bin="$tmpdir/producer-bin"
install -d -m 0755 "$producer_bin"
printf '%s\n' '#!/bin/sh' 'exit 47' > "$producer_bin/tar"
chmod 0755 "$producer_bin/tar"
producer_output="$archive_dir1/overcrow-2.0.0-x86_64-linux.tar.zst"
expect_failure archive-producer 1 'error: could not create archive payload' \
    env PATH="$producer_bin:$PATH" SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" 2.0.0 "$stage1" "$producer_output"

compressor_bin="$tmpdir/compressor-bin"
install -d -m 0755 "$compressor_bin"
printf '%s\n' '#!/bin/sh' 'exit 48' > "$compressor_bin/zstd"
chmod 0755 "$compressor_bin/zstd"
compressor_output="$archive_dir1/overcrow-2.0.1-x86_64-linux.tar.zst"
expect_failure archive-compressor 1 'error: could not compress archive payload' \
    env PATH="$compressor_bin:$PATH" SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" 2.0.1 "$stage1" "$compressor_output"

race_bin="$tmpdir/archive-race-bin"
install -d -m 0755 "$race_bin"
install -m 0755 tests/fixtures/release-mv-race-stub.sh "$race_bin/mv"
race_output="$archive_dir1/overcrow-3.0.0-x86_64-linux.tar.zst"
expect_failure archive-race 1 'error: OUTPUT appeared during publication' \
    env PATH="$race_bin:$PATH" SOURCE_DATE_EPOCH=$source_date_epoch \
        OVERCROW_RACE_DESTINATION="$race_output" \
    "$archive_script" 3.0.0 "$stage1" "$race_output"
grep -Fqx 'racing owner' "$race_output/race-owner" 2>/dev/null ||
    review_failure 'archive publication replaced a racing output'

signal_bin="$tmpdir/archive-signal-bin"
install -d -m 0755 "$signal_bin"
# These expressions belong to the generated signal stub.
# shellcheck disable=SC2016
printf '%s\n' '#!/bin/sh' \
    ': "${OVERCROW_SIGNAL_ARCHIVE_PID:?}"' \
    'kill -TERM "$OVERCROW_SIGNAL_ARCHIVE_PID"' \
    'exit 143' > "$signal_bin/zstd"
chmod 0755 "$signal_bin/zstd"
signal_output="$archive_dir1/overcrow-4.0.0-x86_64-linux.tar.zst"
if PATH="$signal_bin:$PATH" SOURCE_DATE_EPOCH=$source_date_epoch \
        /bin/sh -c '
            OVERCROW_SIGNAL_ARCHIVE_PID=$$
            export OVERCROW_SIGNAL_ARCHIVE_PID
            exec "$@"
        ' sh "$archive_script" 4.0.0 "$stage1" "$signal_output" \
        > "$tmpdir/archive-signal.out" 2> "$tmpdir/archive-signal.err"; then
    review_failure 'signal-interrupted archive creation unexpectedly succeeded'
fi

for output in "$producer_output" "$compressor_output" "$signal_output"; do
    if test -e "$output" || test -L "$output"; then
        review_failure "failed archive operation left an output: $output"
    fi
done
if find "$archive_dir1" -maxdepth 1 -name '.overcrow-archive.*' | grep -q .; then
    review_failure 'failed archive operation left a private temporary'
fi

SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" 0.1.0 "$stage1" "$archive1"
SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" 0.1.0 "$stage2" "$archive2"
cmp "$archive1" "$archive2" ||
    review_failure 'equivalent stages did not produce identical archives'

uncompressed="$tmpdir/overcrow.tar"
zstd --quiet --decompress --stdout "$archive1" > "$uncompressed"
tar --list --file "$uncompressed" > "$tmpdir/archive-entries"
for required in \
        usr/bin/overcrow-control \
        usr/bin/overcrow-core \
        usr/bin/overcrow-overlay \
        usr/lib/overcrow/overcrow-integrate; do
    grep -Fqx "$required" "$tmpdir/archive-entries" ||
        review_failure "archive omitted $required"
done
if grep -Eq 'overcrow-control-host|ControlHost1' "$tmpdir/archive-entries"; then
    review_failure 'archive retained the removed control broker'
fi
if LC_ALL=C grep -a -Eq '(^|[^[:alpha:]])(atime|ctime)=' "$uncompressed"; then
    review_failure 'archive contains nondeterministic atime or ctime fields'
fi
TZ=UTC tar --numeric-owner --full-time --verbose --list \
    --file "$uncompressed" > "$tmpdir/archive-metadata"
if grep -Ev '^[^ ]+ 0/0 +[0-9]+ 2023-11-14 22:13:20 (usr/|usr/.*)$' \
        "$tmpdir/archive-metadata" > "$tmpdir/bad-archive-metadata"; then
    review_failure 'archive owner, group, or mtime was not normalized'
fi

prerelease=0.1.0-pre-alpha.1
arch_version=0.1.0prealpha1
prerelease_archive="$archive_dir1/overcrow-$prerelease-x86_64-linux.tar.zst"
SOURCE_DATE_EPOCH=$source_date_epoch \
    "$archive_script" "$prerelease" "$stage1" "$prerelease_archive"
prerelease_pkgbuild="$tmpdir/PKGBUILD.prerelease"
"$renderer" "$prerelease" "$prerelease_archive" "$prerelease_pkgbuild"
grep -Fqx "pkgver=$arch_version" "$prerelease_pkgbuild" ||
    review_failure 'prerelease PKGBUILD did not normalize the Arch version'
grep -Fqx "_overcrow_version=$prerelease" "$prerelease_pkgbuild" ||
    review_failure 'prerelease PKGBUILD lost the product version'

expect_failure invalid-render-version 2 'error: invalid version' \
    "$renderer" '0.1.0-' "$archive1" "$tmpdir/PKGBUILD.invalid"
expect_failure relative-render-bundle 2 'error: BUNDLE must be absolute' \
    "$renderer" 0.1.0 relative-bundle "$tmpdir/PKGBUILD.relative-bundle"
expect_failure relative-render-output 2 'error: OUTPUT must be absolute' \
    "$renderer" 0.1.0 "$archive1" relative-pkgbuild
wrong_bundle="$tmpdir/wrong-bundle.tar.zst"
cp "$archive1" "$wrong_bundle"
expect_failure invalid-bundle-name 2 \
    'error: BUNDLE filename does not match VERSION' \
    "$renderer" 0.1.0 "$wrong_bundle" "$tmpdir/PKGBUILD.wrong-bundle"

existing_pkgbuild="$tmpdir/PKGBUILD.existing"
printf '%s\n' 'existing PKGBUILD owner' > "$existing_pkgbuild"
expect_failure existing-pkgbuild 2 'error: OUTPUT already exists' \
    "$renderer" 0.1.0 "$archive1" "$existing_pkgbuild"
grep -Fqx 'existing PKGBUILD owner' "$existing_pkgbuild" ||
    review_failure 'renderer replaced an existing output'

checksum_bin="$tmpdir/checksum-bin"
install -d -m 0755 "$checksum_bin"
printf '%s\n' '#!/bin/sh' 'exit 49' > "$checksum_bin/sha256sum"
chmod 0755 "$checksum_bin/sha256sum"
expect_failure checksum-error 1 'error: could not checksum BUNDLE' \
    env PATH="$checksum_bin:$PATH" \
    "$renderer" 0.1.0 "$archive1" "$tmpdir/PKGBUILD.checksum-error"

invalid_checksum_bin="$tmpdir/invalid-checksum-bin"
install -d -m 0755 "$invalid_checksum_bin"
printf '%s\n' '#!/bin/sh' "printf '%s\\n' 'not-a-checksum  ignored'" \
    > "$invalid_checksum_bin/sha256sum"
chmod 0755 "$invalid_checksum_bin/sha256sum"
expect_failure invalid-checksum 1 'error: invalid BUNDLE checksum' \
    env PATH="$invalid_checksum_bin:$PATH" \
    "$renderer" 0.1.0 "$archive1" "$tmpdir/PKGBUILD.invalid-checksum"

render_race_bin="$tmpdir/render-race-bin"
install -d -m 0755 "$render_race_bin"
install -m 0755 tests/fixtures/release-mv-race-stub.sh "$render_race_bin/mv"
render_race_output="$tmpdir/PKGBUILD.race"
expect_failure render-race 1 'error: OUTPUT appeared during publication' \
    env PATH="$render_race_bin:$PATH" \
        OVERCROW_RACE_DESTINATION="$render_race_output" \
    "$renderer" 0.1.0 "$archive1" "$render_race_output"
grep -Fqx 'racing owner' "$render_race_output/race-owner" 2>/dev/null ||
    review_failure 'renderer publication replaced a racing output'

if find "$tmpdir" -maxdepth 1 -name '.overcrow-pkgbuild.*' | grep -q .; then
    review_failure 'renderer failure left a private temporary'
fi

package_dir="$tmpdir/package"
package_output="$tmpdir/packages"
install -d -m 0755 "$package_dir" "$package_output"
bundle="$package_dir/overcrow-0.1.0-x86_64-linux.tar.zst"
cp "$archive1" "$bundle"
"$renderer" 0.1.0 "$bundle" "$package_dir/PKGBUILD"
cp packaging/arch/overcrow.install "$package_dir/overcrow.install"

if command -v makepkg >/dev/null 2>&1; then
    makepkg_home="$tmpdir/makepkg-home"
    makepkg_build="$tmpdir/makepkg-build"
    makepkg_sources="$tmpdir/makepkg-sources"
    install -d -m 0755 \
        "$makepkg_home" \
        "$makepkg_build" \
        "$makepkg_sources"
    (
        cd "$package_dir"
        HOME="$makepkg_home" \
        BUILDDIR="$makepkg_build" \
        PKGDEST="$package_output" \
        SRCDEST="$makepkg_sources" \
        timeout 120 makepkg \
            --clean --cleanbuild --force --noconfirm --nodeps
    ) > "$tmpdir/makepkg.out" 2> "$tmpdir/makepkg.err" ||
        review_failure 'makepkg could not build the rendered package'

    set -- "$package_output"/overcrow-bin-0.1.0-1-x86_64.pkg.tar.*
    if test "$#" -ne 1 || ! test -f "$1"; then
        review_failure 'makepkg did not produce exactly one package'
    elif bsdtar -tf "$1" > "$tmpdir/package-entries"; then
        for required in \
                .PKGINFO \
                usr/bin/overcrow-control \
                usr/bin/overcrow-core \
                usr/bin/overcrow-overlay \
                usr/lib/overcrow/overcrow-integrate; do
            grep -Fqx "$required" "$tmpdir/package-entries" ||
                review_failure "built package omitted $required"
        done
        if grep -Eq 'overcrow-control-host|ControlHost1' "$tmpdir/package-entries"; then
            review_failure 'built package retained the removed control broker'
        fi
    else
        review_failure 'could not inspect the built package'
    fi
fi

if test "$failures" -ne 0; then
    printf '%s\n' "Arch package smoke regressions: $failures" >&2
    exit 1
fi

printf '%s\n' 'Arch PKGBUILD smoke test passed'
