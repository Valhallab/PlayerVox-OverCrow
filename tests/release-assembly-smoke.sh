#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
assembler="$project_root/packaging/release/assemble.sh"
version=0.1.0-pre-alpha.1
arch_version=0.1.0prealpha1
rpm_version=0.1.0.pre_alpha.1
deb_version=0.1.0~pre.alpha.1-1
arch_artifact="overcrow-bin-$arch_version-1-x86_64.pkg.tar.zst"
rpm_artifact="overcrow-$rpm_version-1.fc42.x86_64.rpm"
deb_artifact="overcrow_${deb_version}_amd64.deb"
tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-release-assembly.XXXXXX")

cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    rm -rf -- "$tmpdir"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

source_dir="$tmpdir/source"
output_dir="$tmpdir/release"
mkdir -p "$source_dir"
printf '%s\n' 'Arch package payload' > "$source_dir/$arch_artifact"
printf '%s\n' 'RPM package payload' > "$source_dir/$rpm_artifact"
printf '%s\n' 'DEB package payload' > "$source_dir/$deb_artifact"

"$assembler" "$version" "$source_dir" "$output_dir"

test "$(find "$output_dir" -mindepth 1 -maxdepth 1 -type f | wc -l)" -eq 4
test "$(find "$output_dir" -mindepth 1 -maxdepth 1 -type l | wc -l)" -eq 0
test -f "$output_dir/$arch_artifact"
test -f "$output_dir/$rpm_artifact"
test -f "$output_dir/$deb_artifact"
test -f "$output_dir/SHA256SUMS"
(
    cd "$output_dir"
    sha256sum -c SHA256SUMS >/dev/null
    test "$(sed -n 's/.*  //p' SHA256SUMS)" = \
        "$arch_artifact
$deb_artifact
$rpm_artifact"
    ! grep -Eq '(^|  )/' SHA256SUMS
)

if "$assembler" "$version" "$source_dir" "$output_dir" \
        > "$tmpdir/existing.out" 2> "$tmpdir/existing.err"; then
    printf '%s\n' 'assembler replaced an existing release directory' >&2
    exit 1
fi

missing_source="$tmpdir/missing"
mkdir -p "$missing_source"
if "$assembler" "$version" "$missing_source" "$tmpdir/missing-release" \
        > "$tmpdir/missing.out" 2> "$tmpdir/missing.err"; then
    printf '%s\n' 'assembler accepted a missing package' >&2
    exit 1
fi

missing_rpm_source="$tmpdir/missing-rpm"
mkdir -p "$missing_rpm_source"
cp "$source_dir/$arch_artifact" "$missing_rpm_source/$arch_artifact"
if "$assembler" "$version" "$missing_rpm_source" "$tmpdir/missing-rpm-release" \
        > "$tmpdir/missing-rpm.out" 2> "$tmpdir/missing-rpm.err"; then
    printf '%s\n' 'assembler accepted a missing RPM package' >&2
    exit 1
fi

missing_deb_source="$tmpdir/missing-deb"
mkdir -p "$missing_deb_source"
cp "$source_dir/$arch_artifact" "$missing_deb_source/$arch_artifact"
cp "$source_dir/$rpm_artifact" "$missing_deb_source/$rpm_artifact"
if "$assembler" "$version" "$missing_deb_source" "$tmpdir/missing-deb-release" \
        > "$tmpdir/missing-deb.out" 2> "$tmpdir/missing-deb.err"; then
    printf '%s\n' 'assembler accepted a missing DEB package' >&2
    exit 1
fi

symlink_source="$tmpdir/symlink"
mkdir -p "$symlink_source"
cp "$source_dir/$arch_artifact" "$symlink_source/$arch_artifact"
cp "$source_dir/$rpm_artifact" "$symlink_source/$rpm_artifact"
ln -s -- "$source_dir/$deb_artifact" "$symlink_source/$deb_artifact"
if "$assembler" "$version" "$symlink_source" "$tmpdir/symlink-release" \
        > "$tmpdir/symlink.out" 2> "$tmpdir/symlink.err"; then
    printf '%s\n' 'assembler accepted a symlinked package' >&2
    exit 1
fi

test -z "$(find "$tmpdir" -maxdepth 1 -name '.overcrow-release.*' -print)"
printf '%s\n' 'Release assembly smoke test passed'
