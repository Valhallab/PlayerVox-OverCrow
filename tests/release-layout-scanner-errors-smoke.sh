#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
smoke="$project_root/tests/release-layout-smoke.sh"
tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-scanner-errors.XXXXXX")
trap 'rm -rf -- "$tmpdir"' EXIT HUP INT TERM

expect_scan_error() {
    mode=$1
    command_name=$2
    fixture=$3
    expected_error=$4
    command_dir="$tmpdir/$mode-bin"
    stdout="$tmpdir/$mode.out"
    stderr="$tmpdir/$mode.err"

    install -d -m 0755 "$command_dir"
    install -m 0755 "$fixture" "$command_dir/$command_name"

    if PATH="$command_dir:$PATH" \
            OVERCROW_GREP_ERROR_MODE="$mode" \
            OVERCROW_FIND_ERROR_MODE="$mode" \
            "$smoke" > "$stdout" 2> "$stderr"; then
        printf '%s\n' "scanner error was masked: $mode" >&2
        return 1
    fi
    if ! grep -Fqx "$expected_error" "$stderr"; then
        printf '%s\n' "scanner error did not fail closed: $mode" >&2
        sed -n '1,120p' "$stderr" >&2
        return 1
    fi
}

cd "$project_root"
failures=0
expect_scan_error token grep tests/fixtures/release-grep-error-stub.sh \
    'could not scan staged runtime for unresolved build references' || failures=$((failures + 1))
expect_scan_error private grep tests/fixtures/release-grep-error-stub.sh \
    'could not scan staged runtime for legacy repository URLs' || failures=$((failures + 1))
expect_scan_error source find tests/fixtures/release-find-error-stub.sh \
    'could not enumerate staged runtime source files' || failures=$((failures + 1))
test "$failures" -eq 0
