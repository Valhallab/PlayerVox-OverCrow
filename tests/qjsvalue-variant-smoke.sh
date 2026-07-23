#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
if ! command -v pkg-config >/dev/null 2>&1 || ! pkg-config --exists Qt6Qml; then
    printf '%s\n' "Qt6Qml development runtime unavailable; QJSEngine QVariant smoke test skipped"
    exit 0
fi

build_dir=$(mktemp -d)
trap '/usr/bin/rm -rf "$build_dir"' EXIT HUP INT TERM

# pkg-config intentionally expands compiler flags into separate arguments.
# shellcheck disable=SC2046
c++ -std=c++20 "$repo_root/tests/qjsvalue-variant-smoke.cpp" \
    -o "$build_dir/qjsvalue-variant-smoke" \
    $(pkg-config --cflags --libs Qt6Qml)
"$build_dir/qjsvalue-variant-smoke" "$repo_root/integrations/kwin/contents/code/main.js"
