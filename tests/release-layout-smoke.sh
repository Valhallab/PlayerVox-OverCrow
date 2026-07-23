#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
stage_script="$project_root/packaging/release/stage.sh"
manifest="$project_root/packaging/release/manifest.txt"
cd "$project_root"

test -x "$stage_script"
test -f "$manifest"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-stage.XXXXXX")
trap 'rm -rf -- "$tmpdir"' EXIT HUP INT TERM
notices="$tmpdir/THIRD_PARTY_LICENSES.md"
stage="$tmpdir/stage"
source_date_epoch=1700000000
review_failures=0

review_failure() {
    printf '%s\n' "$1" >&2
    review_failures=$((review_failures + 1))
}

scripts/generate-third-party-notices.sh "$notices"

legacy_repo_pattern='github\.com(:[0-9]+)?[:/](matthieugc/overcrow|overcrow/overcrow)(\.git)?([^[:alnum:]_.-]|$)'
repository_url_sentinel="$tmpdir/repository-url-sentinel"
while IFS= read -r legacy_url; do
    printf '%s\n' "$legacy_url" > "$repository_url_sentinel"
    if ! grep -a -i -Eq "$legacy_repo_pattern" "$repository_url_sentinel"; then
        review_failure "legacy repository URL guard missed sentinel: $legacy_url"
    fi
done <<'EOF'
https://github.com/overcrow/overcrow
https://github.com/MatthieuGC/Overcrow
https://github.com/MatthieuGC/Overcrow.git
EOF
while IFS= read -r public_url; do
    printf '%s\n' "$public_url" > "$repository_url_sentinel"
    if grep -a -i -Eq "$legacy_repo_pattern" "$repository_url_sentinel"; then
        review_failure "legacy repository URL guard rejected public sentinel: $public_url"
    fi
done <<'EOF'
https://github.com/Valhallab/PlayerVox-OverCrow
https://github.com/Valhallab/PlayerVox-OverCrow.git
https://github.com/emilk/egui
https://github.com/MatthieuGC/OvercrowTools
https://github.com/overcrow/overcrow-tools
EOF

printf '%s\n' '# relative notice sentinel' > "$tmpdir/relative-notices"
bypass_destination="$tmpdir/colon:/relative-bypass"
if (
    cd "$tmpdir"
    "$stage_script" "$bypass_destination" relative-notices
) > "$tmpdir/relative-bypass.out" 2> "$tmpdir/relative-bypass.err"; then
    bypass_status=0
else
    bypass_status=$?
fi
if test "$bypass_status" -ne 2 ||
        ! grep -Fqx 'error: both paths must be absolute' "$tmpdir/relative-bypass.err"; then
    review_failure 'stage accepted a relative notice path hidden by a colon-slash destination'
fi
if test -e "$bypass_destination"; then
    review_failure 'relative-path rejection left a destination behind'
fi

make_fake_project() {
    fake_root=$1
    install -d -m 0755 "$fake_root/packaging/release" "$fake_root/target/release"
    install -m 0755 "$stage_script" "$fake_root/packaging/release/stage.sh"
    for fake_binary in overcrow-control overcrow-core overcrow-hyprland overcrow-overlay overcrowctl; do
        install -m 0755 /bin/true "$fake_root/target/release/$fake_binary"
    done
}

non_elf_root="$tmpdir/non-elf-project"
make_fake_project "$non_elf_root"
printf '%s\n' '#!/bin/sh' 'exit 0' > "$non_elf_root/target/release/overcrow-control"
chmod 0755 "$non_elf_root/target/release/overcrow-control"
non_elf_destination="$tmpdir/non-elf-stage"
if "$non_elf_root/packaging/release/stage.sh" \
        "$non_elf_destination" "$notices" \
        > "$tmpdir/non-elf.out" 2> "$tmpdir/non-elf.err"; then
    non_elf_status=0
else
    non_elf_status=$?
fi
if test "$non_elf_status" -ne 1 ||
        ! grep -Fqx 'error: cannot inspect release binary: overcrow-control' \
            "$tmpdir/non-elf.err"; then
    review_failure 'stage did not explicitly reject a non-ELF release binary'
fi
if test -e "$non_elf_destination"; then
    review_failure 'non-ELF rejection left a destination behind'
fi

wrong_arch_root="$tmpdir/wrong-arch-project"
make_fake_project "$wrong_arch_root"
printf '\267\000' | dd of="$wrong_arch_root/target/release/overcrow-control" \
    bs=1 seek=18 conv=notrunc 2>/dev/null
wrong_arch_destination="$tmpdir/wrong-arch-stage"
if "$wrong_arch_root/packaging/release/stage.sh" \
        "$wrong_arch_destination" "$notices" \
        > "$tmpdir/wrong-arch.out" 2> "$tmpdir/wrong-arch.err"; then
    wrong_arch_status=0
else
    wrong_arch_status=$?
fi
if test "$wrong_arch_status" -ne 1 ||
        ! grep -Fqx 'error: release binary is not ELF64 x86-64: overcrow-control' \
            "$tmpdir/wrong-arch.err"; then
    review_failure 'stage did not explicitly reject a wrong-architecture ELF binary'
fi
if test -e "$wrong_arch_destination"; then
    review_failure 'wrong-architecture rejection left a destination behind'
fi

invalid_epoch_destination="$tmpdir/invalid-epoch-stage"
if SOURCE_DATE_EPOCH=invalid "$stage_script" \
        "$invalid_epoch_destination" "$notices" \
        > "$tmpdir/invalid-epoch.out" 2> "$tmpdir/invalid-epoch.err"; then
    invalid_epoch_status=0
else
    invalid_epoch_status=$?
fi
if test "$invalid_epoch_status" -eq 0; then
    review_failure 'stage accepted an invalid SOURCE_DATE_EPOCH'
fi
if test -e "$invalid_epoch_destination"; then
    review_failure 'invalid SOURCE_DATE_EPOCH left a destination behind'
fi

token_notices="$tmpdir/token-notices"
printf '%s\n' '# OverCrow third-party software notices' '@OVERCROW_BINDIR@' \
    > "$token_notices"
token_destination="$tmpdir/token-stage"
if "$stage_script" "$token_destination" "$token_notices" \
        > "$tmpdir/token.out" 2> "$tmpdir/token.err"; then
    token_status=0
else
    token_status=$?
fi
if test "$token_status" -ne 1 ||
        ! grep -Fqx 'error: staged runtime contains unresolved binary-directory token' \
            "$tmpdir/token.err"; then
    review_failure 'stage did not reject an unresolved binary-directory token before publication'
fi
if test -e "$token_destination"; then
    review_failure 'unresolved-token rejection left a destination behind'
fi

path_notices="$tmpdir/path-notices"
printf '%s\n' '# OverCrow third-party software notices' "$project_root" \
    > "$path_notices"
path_destination="$tmpdir/path-stage"
if "$stage_script" "$path_destination" "$path_notices" \
        > "$tmpdir/path.out" 2> "$tmpdir/path.err"; then
    path_status=0
else
    path_status=$?
fi
if test "$path_status" -ne 1 ||
        ! grep -Fqx 'error: staged runtime contains private workspace path' \
            "$tmpdir/path.err"; then
    review_failure 'stage did not reject a workspace path in a staged notice'
fi
if test -e "$path_destination"; then
    review_failure 'workspace-path rejection left a destination behind'
fi

symlink_root="$tmpdir/checkout-link"
ln -s "$project_root" "$symlink_root"
symlink_path_destination="$tmpdir/symlink-path-stage"
if "$symlink_root/packaging/release/stage.sh" \
        "$symlink_path_destination" "$path_notices" \
        > "$tmpdir/symlink-path.out" 2> "$tmpdir/symlink-path.err"; then
    symlink_path_status=0
else
    symlink_path_status=$?
fi
if test "$symlink_path_status" -ne 1 ||
        ! grep -Fqx 'error: staged runtime contains private workspace path' \
            "$tmpdir/symlink-path.err"; then
    review_failure 'stage invoked through a symlink did not scan the physical checkout path'
fi
if test -e "$symlink_path_destination"; then
    review_failure 'symlink-path rejection left a destination behind'
fi

signal_bin="$tmpdir/signal-bin"
install -d -m 0755 "$signal_bin"
install -m 0755 tests/fixtures/release-install-signal-stub.sh "$signal_bin/install"
signal_destination="$tmpdir/signal-stage"
PATH="$signal_bin:$PATH" /bin/sh -c '
    OVERCROW_SIGNAL_STAGE_PID=$$
    export OVERCROW_SIGNAL_STAGE_PID
    exec "$@"
' sh "$stage_script" "$signal_destination" "$notices" \
    > "$tmpdir/signal.out" 2> "$tmpdir/signal.err" &
signal_pid=$!
if wait "$signal_pid"; then
    signal_status=0
else
    signal_status=$?
fi
if test "$signal_status" -eq 0; then
    review_failure 'signal-injected staging unexpectedly succeeded'
fi
if test -e "$signal_destination"; then
    review_failure 'signal interruption left a partial destination behind'
fi
if find "$tmpdir" -maxdepth 1 -name '.overcrow-stage.*' | grep -q .; then
    review_failure 'signal interruption left a private staging directory behind'
fi

race_bin="$tmpdir/race-bin"
install -d -m 0755 "$race_bin"
install -m 0755 tests/fixtures/release-mv-race-stub.sh "$race_bin/mv"
race_destination="$tmpdir/race-stage"
if PATH="$race_bin:$PATH" OVERCROW_RACE_DESTINATION="$race_destination" \
        "$stage_script" "$race_destination" "$notices" \
        > "$tmpdir/race.out" 2> "$tmpdir/race.err"; then
    race_status=0
else
    race_status=$?
fi
if test "$race_status" -ne 1; then
    review_failure 'stage published despite a destination creation race'
fi
if ! grep -Fqx 'racing owner' "$race_destination/race-owner" 2>/dev/null; then
    review_failure 'stage replaced or absorbed the racing destination'
fi
if test -e "$race_destination/usr"; then
    review_failure 'stage merged payload files into the racing destination'
fi
if find "$tmpdir" -maxdepth 1 -name '.overcrow-stage.*' | grep -q .; then
    review_failure 'publication race left a private staging directory behind'
fi

dangling_bin="$tmpdir/dangling-bin"
install -d -m 0755 "$dangling_bin"
install -m 0755 tests/fixtures/release-mv-dangling-stub.sh "$dangling_bin/mv"
dangling_destination="$tmpdir/dangling-stage"
dangling_hold="$tmpdir/dangling-hold"
if PATH="$dangling_bin:$PATH" OVERCROW_DANGLING_HOLD="$dangling_hold" \
        "$stage_script" "$dangling_destination" "$notices" \
        > "$tmpdir/dangling.out" 2> "$tmpdir/dangling.err"; then
    dangling_status=0
else
    dangling_status=$?
fi
if test "$dangling_status" -ne 1; then
    review_failure 'dangling-symlink publication failure unexpectedly succeeded'
fi
if test -e "$dangling_destination" || test -L "$dangling_destination"; then
    review_failure 'dangling-symlink publication failure left a destination behind'
fi
dangling_leak=$(find "$tmpdir" -maxdepth 1 -name '.overcrow-stage.*' -print -quit)
if test -n "$dangling_leak"; then
    review_failure 'publication failure left a dangling private staging path behind'
fi

if test "$review_failures" -ne 0; then
    printf '%s\n' "release staging review regressions: $review_failures" >&2
    exit 1
fi

SOURCE_DATE_EPOCH=$source_date_epoch "$stage_script" "$stage" "$notices"

(cd "$stage" && find . -type f -printf '%P\n' | LC_ALL=C sort) \
    > "$tmpdir/actual-files.txt"
cmp "$manifest" "$tmpdir/actual-files.txt"

cat > "$tmpdir/expected-directories.txt" <<'EOF'
usr
usr/bin
usr/lib
usr/lib/overcrow
usr/lib/systemd
usr/lib/systemd/user
usr/share
usr/share/applications
usr/share/icons
usr/share/icons/hicolor
usr/share/icons/hicolor/512x512
usr/share/icons/hicolor/512x512/apps
usr/share/licenses
usr/share/licenses/overcrow
usr/share/metainfo
usr/share/overcrow
usr/share/overcrow/integrations
usr/share/overcrow/integrations/hyprland
usr/share/overcrow/integrations/kwin
usr/share/overcrow/integrations/kwin/contents
usr/share/overcrow/integrations/kwin/contents/code
EOF

verify_stage_tree() {
    candidate=$1

    (cd "$candidate" && find . -type f -printf '%P\n' | LC_ALL=C sort) \
        > "$tmpdir/verifier-files.txt" || return 1
    cmp "$manifest" "$tmpdir/verifier-files.txt" >/dev/null || return 1

    (cd "$candidate" && \
        find . -mindepth 1 -type d -printf '%P\n' | LC_ALL=C sort) \
        > "$tmpdir/verifier-directories.txt" || return 1
    cmp "$tmpdir/expected-directories.txt" \
        "$tmpdir/verifier-directories.txt" >/dev/null || return 1

    special_entry=$(find "$candidate" ! \( -type d -o -type f \) -print -quit)
    test -z "$special_entry" || return 1

    bad_directory_mode=$(find "$candidate" -type d ! -perm 0755 -print -quit)
    test -z "$bad_directory_mode" || return 1

    while IFS= read -r staged_path; do
        case $staged_path in
            usr/bin/*|usr/lib/overcrow/overcrow-integrate) expected_mode=755 ;;
            *) expected_mode=644 ;;
        esac
        actual_mode=$(stat -c '%a' "$candidate/$staged_path") || return 1
        test "$actual_mode" = "$expected_mode" || return 1
    done < "$manifest"

    if ! mtime_error=$(find "$candidate" -exec sh -c '
        expected_epoch=$1
        shift
        for entry do
            actual_epoch=$(stat -c "%Y" "$entry") || exit 1
            if test "$actual_epoch" != "$expected_epoch"; then
                printf "%s\n" "$entry"
                exit 1
            fi
        done
    ' sh "$source_date_epoch" {} +); then
        printf '%s\n' "$mtime_error" >&2
        return 1
    fi
}

verify_stage_tree "$stage"

install -d -m 0755 "$stage/unexpected-root-directory"
find "$stage" -exec touch -h -d "@$source_date_epoch" {} +
if verify_stage_tree "$stage" > /dev/null 2>&1; then
    printf '%s\n' 'stage-tree verifier accepted an extra root directory' >&2
    exit 1
fi
rmdir "$stage/unexpected-root-directory"
find "$stage" -exec touch -h -d "@$source_date_epoch" {} +

printf '%s\n' 'unexpected root file' > "$stage/unexpected-root-file"
chmod 0644 "$stage/unexpected-root-file"
find "$stage" -exec touch -h -d "@$source_date_epoch" {} +
if verify_stage_tree "$stage" > /dev/null 2>&1; then
    printf '%s\n' 'stage-tree verifier accepted an extra root file' >&2
    exit 1
fi
rm -f -- "$stage/unexpected-root-file"
find "$stage" -exec touch -h -d "@$source_date_epoch" {} +

install -d -m 0755 "$stage/usr/share/unexpected-directory"
if verify_stage_tree "$stage" > /dev/null 2>&1; then
    printf '%s\n' 'stage-tree verifier accepted an extra directory' >&2
    exit 1
fi
rmdir "$stage/usr/share/unexpected-directory"

ln -s overcrow "$stage/usr/share/unexpected-link"
if verify_stage_tree "$stage" > /dev/null 2>&1; then
    printf '%s\n' 'stage-tree verifier accepted a symlink' >&2
    exit 1
fi
rm -f -- "$stage/usr/share/unexpected-link"

mkfifo "$stage/usr/share/unexpected-fifo"
if verify_stage_tree "$stage" > /dev/null 2>&1; then
    printf '%s\n' 'stage-tree verifier accepted a special file' >&2
    exit 1
fi
rm -f -- "$stage/usr/share/unexpected-fifo"

chmod 0700 "$stage/usr/share"
if verify_stage_tree "$stage" > /dev/null 2>&1; then
    printf '%s\n' 'stage-tree verifier accepted an invalid directory mode' >&2
    exit 1
fi
chmod 0755 "$stage/usr/share"

chmod 0600 "$stage/usr/share/overcrow/TRADEMARKS.md"
if verify_stage_tree "$stage" > /dev/null 2>&1; then
    printf '%s\n' 'stage-tree verifier accepted an invalid file mode' >&2
    exit 1
fi
chmod 0644 "$stage/usr/share/overcrow/TRADEMARKS.md"

touch "$stage/usr/share/overcrow/TRADEMARKS.md"
if verify_stage_tree "$stage" > /dev/null 2>&1; then
    printf '%s\n' 'stage-tree verifier accepted a nondeterministic mtime' >&2
    exit 1
fi
find "$stage" -exec touch -h -d "@$source_date_epoch" {} +
verify_stage_tree "$stage"

while IFS= read -r binary; do
    staged_binary="$stage/usr/bin/$binary"
    test -x "$staged_binary"
    if ! elf_report=$(LC_ALL=C readelf --file-header --program-headers --wide \
            "$staged_binary" 2>&1); then
        printf '%s\n' "cannot inspect staged release binary: $binary" >&2
        exit 1
    fi
    if ! printf '%s\n' "$elf_report" | grep -Eq '^  Class:[[:space:]]+ELF64$' ||
            ! printf '%s\n' "$elf_report" |
                grep -Eq '^  Machine:[[:space:]]+Advanced Micro Devices X86-64$' ||
            ! printf '%s\n' "$elf_report" |
                grep -Eq 'Requesting program interpreter: /(lib64|usr/lib)/ld-linux-x86-64\.so\.2]'; then
        printf '%s\n' "staged release binary is not ELF64 x86-64 GNU/Linux: $binary" >&2
        exit 1
    fi
    if ! section_report=$(LC_ALL=C readelf --sections --wide "$staged_binary" 2>&1); then
        printf '%s\n' "cannot inspect staged release binary sections: $binary" >&2
        exit 1
    fi
    if printf '%s\n' "$section_report" | grep -Eq '\.debug_|\.symtab'; then
        printf '%s\n' "release binary contains debug or static symbol sections: $binary" >&2
        exit 1
    fi
done <<'EOF'
overcrow-control
overcrow-core
overcrow-hyprland
overcrow-overlay
overcrowctl
EOF

grep -Fq 'GNU AFFERO GENERAL PUBLIC LICENSE' \
    "$stage/usr/share/licenses/overcrow/LICENSE"
grep -Fqx \
    'OverCrow was originally created by Valhallab SASU and distributed under the PlayerVox brand.' \
    "$stage/usr/share/licenses/overcrow/NOTICE"
grep -Fqx '# OverCrow third-party software notices' \
    "$stage/usr/share/licenses/overcrow/THIRD_PARTY_LICENSES.md"

if unresolved_matches=$(grep -R -a -n -E \
        '@OVERCROW_BINDIR@|%h/\.local/bin' "$stage"); then
    printf '%s\n' "$unresolved_matches" >&2
    printf '%s\n' 'staged runtime contains an unresolved build reference' >&2
    exit 1
else
    unresolved_status=$?
fi
if test "$unresolved_status" -ne 1; then
    printf '%s\n' \
        'could not scan staged runtime for unresolved build references' >&2
    exit 1
fi

if workspace_matches=$(grep -R -a -n -F "$project_root" "$stage"); then
    printf '%s\n' "$workspace_matches" >&2
    printf '%s\n' 'staged runtime contains a private workspace path' >&2
    exit 1
else
    workspace_status=$?
fi
if test "$workspace_status" -ne 1; then
    printf '%s\n' 'could not scan staged runtime for private workspace paths' >&2
    exit 1
fi

if legacy_repo_matches=$(grep -R -a -i -n -E \
        "$legacy_repo_pattern" "$stage"); then
    printf '%s\n' "$legacy_repo_matches" >&2
    printf '%s\n' 'staged runtime contains a legacy OverCrow repository URL' >&2
    exit 1
else
    legacy_repo_status=$?
fi
if test "$legacy_repo_status" -ne 1; then
    printf '%s\n' 'could not scan staged runtime for legacy repository URLs' >&2
    exit 1
fi

if ! source_files=$(find "$stage" -type f \
        \( -name '*.rs' -o -name 'Cargo.toml' -o -name 'Cargo.lock' \)); then
    printf '%s\n' 'could not enumerate staged runtime source files' >&2
    exit 1
fi
if test -n "$source_files"; then
    printf '%s\n' "$source_files" >&2
    printf '%s\n' 'staged runtime contains source files' >&2
    exit 1
fi

test ! -e "$stage/usr/share/kwin"
if find "$tmpdir" -maxdepth 1 -name '.overcrow-stage.*' | grep -q .; then
    printf '%s\n' 'successful publication left a private staging directory behind' >&2
    exit 1
fi
