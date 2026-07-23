#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

test -f README.md
test -f docs/architecture.md
test -f docs/troubleshooting.md
test -f docs/testing/manual-mvp.md
test -f SECURITY.md
test ! -e docs/superpowers

readme_lines=$(wc -l < README.md)
test "$readme_lines" -le 220

for heading in \
    '## Compatibility' \
    '## Quick start' \
    '## Using OverCrow' \
    '## Built-in widgets' \
    '## Safety' \
    '## Limitations' \
    '## Development' \
    '## License'
do
    grep -Fqx "$heading" README.md
done

grep -Fq 'https://github.com/Valhallab/PlayerVox-OverCrow' README.md
grep -Fq 'yay -S overcrow-bin' README.md
grep -Fq \
    'https://github.com/Valhallab/PlayerVox-OverCrow/releases/tag/v0.1.0-pre-alpha.2' \
    README.md
if grep -Fq 'No AUR package or prebuilt GitHub release is published yet.' README.md; then
    printf '%s\n' 'README still claims that no public release exists' >&2
    exit 1
fi
grep -Fq 'docs/architecture.md' README.md
grep -Fq 'docs/troubleshooting.md' README.md
grep -Fq 'SECURITY.md' README.md

if grep -E -i -n \
        'authorized source checkout|github.com/(MatthieuGC/Overcrow|overcrow/overcrow)' \
        README.md docs/architecture.md docs/troubleshooting.md SECURITY.md; then
    printf '%s\n' 'public documentation contains private-era wording or URLs' >&2
    exit 1
fi

if grep -Fq 'docs/superpowers' AGENTS.md; then
    printf '%s\n' 'agent guidance references private development records' >&2
    exit 1
fi
