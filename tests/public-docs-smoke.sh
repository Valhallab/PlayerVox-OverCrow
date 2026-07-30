#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

test -f README.md
test -f docs/architecture.md
test -f docs/troubleshooting.md
test -f docs/testing/manual-mvp.md
test -f SECURITY.md
test -f .github/ISSUE_TEMPLATE/bug-report.yml
test ! -e docs/superpowers

readme_lines=$(wc -l < README.md)
test "$readme_lines" -le 250

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
grep -Fq 'On Fedora 43 or 44' README.md
grep -Fq 'dnf copr enable grmpy/playervox-overcrow' README.md
grep -Fq 'dnf5 copr enable grmpy/playervox-overcrow' README.md
grep -Fq 'Ubuntu 24.04' README.md
grep -Fq \
    'sudo apt install ./overcrow_0.1.0~pre.alpha.4-1_amd64.deb' \
    README.md
grep -Fq 'one Ubuntu-baseline DEB' README.md
grep -Fq \
    'https://github.com/Valhallab/PlayerVox-OverCrow/releases/tag/v0.1.0-pre-alpha.4' \
    README.md
if grep -Fq 'No AUR package or prebuilt GitHub release is published yet.' README.md; then
    printf '%s\n' 'README still claims that no public release exists' >&2
    exit 1
fi
grep -Fq 'docs/architecture.md' README.md
grep -Fq 'docs/troubleshooting.md' README.md
grep -Fq 'SECURITY.md' README.md
grep -Fq 'Report a problem' README.md
grep -Fq 'No GitHub account is required.' README.md
grep -Fiq 'uploaded automatically' docs/troubleshooting.md
grep -Fq 'fixed PlayerVox support endpoint over HTTPS' docs/troubleshooting.md
grep -Fq 'Support reference or copied report' .github/ISSUE_TEMPLATE/bug-report.yml
if grep -R -F -n 'overcrow-support-report.md' \
        README.md docs .github/ISSUE_TEMPLATE; then
    printf '%s\n' 'documentation still references the removed local report file' >&2
    exit 1
fi

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
