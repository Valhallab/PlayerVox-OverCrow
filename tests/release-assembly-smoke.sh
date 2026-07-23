#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
assembler="$project_root/packaging/release/assemble.sh"
version=0.1.0-pre-alpha.1
arch_version=0.1.0prealpha1
artifact="overcrow-bin-$arch_version-1-x86_64.pkg.tar.zst"
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
printf '%s\n' 'package payload' > "$source_dir/$artifact"

"$assembler" "$version" "$source_dir" "$output_dir"

test "$(find "$output_dir" -mindepth 1 -maxdepth 1 -type f | wc -l)" -eq 2
test "$(find "$output_dir" -mindepth 1 -maxdepth 1 -type l | wc -l)" -eq 0
test -f "$output_dir/$artifact"
test -f "$output_dir/SHA256SUMS"
(
    cd "$output_dir"
    sha256sum -c SHA256SUMS >/dev/null
    test "$(sed -n 's/.*  //p' SHA256SUMS)" = "$artifact"
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

symlink_source="$tmpdir/symlink"
mkdir -p "$symlink_source"
ln -s -- "$source_dir/$artifact" "$symlink_source/$artifact"
if "$assembler" "$version" "$symlink_source" "$tmpdir/symlink-release" \
        > "$tmpdir/symlink.out" 2> "$tmpdir/symlink.err"; then
    printf '%s\n' 'assembler accepted a symlinked package' >&2
    exit 1
fi

test -z "$(find "$tmpdir" -maxdepth 1 -name '.overcrow-release.*' -print)"
printf '%s\n' 'Release assembly smoke test passed'
