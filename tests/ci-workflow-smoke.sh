#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
workflow="$project_root/.github/workflows/ci.yml"

test -f "$workflow"
grep -Fq 'contents: read' "$workflow"
grep -Fq 'actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683' "$workflow"
grep -Fq 'timeout-minutes: 25' "$workflow"
grep -Fq 'cargo fmt --all -- --check' "$workflow"
grep -Fq 'cargo clippy --workspace --all-targets -- -D warnings' "$workflow"
grep -Fq 'cargo test --workspace --all-targets --locked' "$workflow"
grep -Fq 'actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020' "$workflow"
grep -Fq 'libwebkit2gtk-4.1-dev' "$workflow"
grep -Fq 'fontconfig' "$workflow"
grep -Fq 'npm ci --ignore-scripts' "$workflow"
grep -Fq 'npm test' "$workflow"
grep -Fq 'npm run build' "$workflow"
grep -Fq 'shellcheck scripts/*.sh scripts/lib/*.sh tests/*.sh' "$workflow"
grep -Fq 'packaging/arch/*.install packaging/arch/*.sh packaging/aur/*.install' "$workflow"
grep -Fq 'shellcheck -s bash packaging/aur/PKGBUILD' "$workflow"
grep -Fq 'sh -n scripts/*.sh scripts/lib/*.sh tests/*.sh' "$workflow"
grep -Fq 'bash -n packaging/aur/PKGBUILD' "$workflow"
grep -Fq 'node --check integrations/kwin/contents/code/main.js' "$workflow"
grep -Fq 'node --test tests/kwin-bridge.test.js' "$workflow"
grep -Fq 'tests/qjsvalue-variant-smoke.sh' "$workflow"
grep -Fq 'tests/aur-package-smoke.sh' "$workflow"
grep -Fq 'tests/public-docs-smoke.sh' "$workflow"
grep -Fq 'tests/public-license-policy-smoke.sh' "$workflow"
grep -Fq 'tests/native-only-distribution-smoke.sh' "$workflow"

if grep -Eq \
        'pull_request_target|docker|podman|cargo install|cargo deny|cargo build --workspace --release|for smoke_test' \
        "$workflow"; then
    printf '%s\n' 'CI uses an unsafe trigger or nonessential hosted release work' >&2
    exit 1
fi

if grep -Eq '^[[:space:]]*done[[:space:]]*$' "$workflow"; then
    printf '%s\n' 'CI contains an orphaned shell loop terminator' >&2
    exit 1
fi
