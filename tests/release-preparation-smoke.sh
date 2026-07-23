#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
preparer="$project_root/scripts/prepare-release.sh"
tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-release-policy.XXXXXX")

cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    rm -rf -- "$tmpdir"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

test -x "$preparer"

if "$preparer" unexpected > "$tmpdir/args.out" 2> "$tmpdir/args.err"; then
    printf '%s\n' 'release preparer accepted an argument' >&2
    exit 1
fi

stub_bin="$tmpdir/bin"
mkdir -p "$stub_bin"
{
    printf '%s\n' '#!/bin/sh' 'set -eu'
    printf '%s\n' 'case "$*" in'
    printf '%s\n' \
        '  "symbolic-ref --quiet --short HEAD") printf "%s\n" feature/test ;;' \
        '  *) exit 1 ;;' \
        'esac'
} > "$stub_bin/git"
chmod 0755 "$stub_bin/git"
if PATH="$stub_bin:$PATH" "$preparer" \
        > "$tmpdir/branch.out" 2> "$tmpdir/branch.err"; then
    printf '%s\n' 'release preparer accepted a non-master branch' >&2
    exit 1
fi
grep -Fq 'branch master' "$tmpdir/branch.err"

# These are intentional literal command shapes read from the target script.
# shellcheck disable=SC2016
for required in \
        'git symbolic-ref --quiet --short HEAD' \
        'git status --porcelain --untracked-files=all' \
        'git show -s --format=%ct HEAD' \
        'npm --prefix "$project_root/crates/overcrow-control-ui" ci' \
        'npm --prefix "$project_root/crates/overcrow-control-ui" test' \
        'npm --prefix "$project_root/crates/overcrow-control-ui" run build' \
        'cargo fmt --all -- --check' \
        'cargo clippy --workspace --all-targets -- -D warnings' \
        'cargo test --workspace --all-targets --locked' \
        'cargo deny --locked check advisories licenses' \
        'cargo deny --locked check bans sources' \
        '"$project_root/scripts/build-arch-package.sh"' \
        '"$project_root/packaging/release/inspect.sh"' \
        '"$project_root/packaging/release/assemble.sh"'; do
    grep -Fq "$required" "$preparer"
done

if grep -Eiq \
        'sudo|git[[:space:]]+(tag|push)|(^|[^[:alnum:]_])gh([^[:alnum:]_]|$)|aur@|systemctl|^[[:space:]]*overcrow-(control|core|overlay|hyprland|ctl)([[:space:]]|$)' \
        "$preparer"; then
    printf '%s\n' 'release preparer contains a forbidden mutation path' >&2
    exit 1
fi

printf '%s\n' 'Release preparation policy smoke test passed'
