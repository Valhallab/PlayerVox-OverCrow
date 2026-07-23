#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

deny_version=$(cargo deny --version 2>/dev/null) || {
    printf '%s\n' 'error: cargo-deny 0.19.4 is required' >&2
    exit 1
}
test "$deny_version" = 'cargo-deny 0.19.4' || {
    printf '%s\n' 'error: cargo-deny 0.19.4 is required' >&2
    exit 1
}

cargo deny --locked --offline --target x86_64-unknown-linux-gnu check licenses
