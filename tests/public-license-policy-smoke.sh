#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

test -f LICENSE
test -f NOTICE
test -f TRADEMARKS.md
test -f CONTRIBUTING.md
test -f SECURITY.md

grep -Fq 'GNU AFFERO GENERAL PUBLIC LICENSE' LICENSE
grep -Fq 'Version 3, 19 November 2007' LICENSE
grep -Fq 'END OF TERMS AND CONDITIONS' LICENSE
test "$(wc -l < LICENSE)" -gt 200

grep -Fqx \
    'OverCrow was originally created by Valhallab SASU and distributed under the PlayerVox brand.' \
    NOTICE
grep -Fq 'Copyright (c) 2026 Valhallab SASU' NOTICE
grep -Fq 'AGPL-3.0-only' NOTICE
grep -Fq 'Tabler Icons 3.45.0' NOTICE
grep -Fq 'Copyright (c) 2020-2026 Paweł Kuna' NOTICE
grep -Fq 'Discord and Twitch brand marks' NOTICE
test -s assets/icons/tabler-icons-overcrow.ttf
test -s assets/icons/discord-symbol-blurple.png
test -s assets/icons/twitch-glitch-purple.png

grep -Fq 'PlayerVox is a registered trademark owned by Valhallab SASU.' TRADEMARKS.md
grep -Fq 'modified distribution' TRADEMARKS.md
grep -Fq 'AGPL-3.0-only' CONTRIBUTING.md
grep -Fq 'AGPL-3.0-only' README.md
grep -Fq 'Valhallab SASU' README.md

metadata=$(cargo metadata --locked --offline --no-deps --format-version 1)
printf '%s\n' "$metadata" | jq -e --arg root "$project_root" '
    [.packages[] | select(.manifest_path | startswith($root + "/crates/"))] as $packages
    | ($packages | length) >= 5
    and all($packages[];
        .authors == ["Valhallab SASU"]
        and .license == "AGPL-3.0-only"
        and .license_file == null
        and .publish == []
    )
' >/dev/null

jq -e '
    .KPlugin.Authors == [{"Name": "Valhallab SASU"}]
    and .KPlugin.License == "AGPL-3.0-only"
    and .KPlugin.Copyright == "Copyright (c) 2026 Valhallab SASU"
    and .KPlugin.Website == "https://github.com/Valhallab/PlayerVox-OverCrow"
' integrations/kwin/metadata.json >/dev/null

grep -Fq \
    'https://github.com/Valhallab/PlayerVox-OverCrow/security/advisories/new' \
    SECURITY.md

grep -Fq 'Name=PlayerVox OverCrow' \
    packaging/applications/com.playervox.OverCrow.desktop
grep -Fq 'Exec=overcrow-control' \
    packaging/applications/com.playervox.OverCrow.desktop
grep -Fq '<name>PlayerVox OverCrow</name>' \
    packaging/metainfo/com.playervox.OverCrow.metainfo.xml
grep -Fq '<title>PlayerVox OverCrow</title>' \
    crates/overcrow-control-ui/index.html
grep -Fq 'PlayerVox OverCrow was installed inertly.' \
    packaging/arch/overcrow.install
jq -e '
    .productName == "PlayerVox OverCrow"
    and .app.windows[0].title == "PlayerVox OverCrow"
' crates/overcrow-control-ui/src-tauri/tauri.conf.json >/dev/null

npm_notice_dir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-npm-notices.XXXXXX")
trap 'rm -rf -- "$npm_notice_dir"' EXIT HUP INT TERM
(
    cd crates/overcrow-control-ui
    NO_COLOR=1 npm ls --omit=dev --all --json --long
) > "$npm_notice_dir/tree.json"
scripts/generate-npm-notices.mjs \
    "$npm_notice_dir/tree.json" "$npm_notice_dir/notices.md"
grep -Fq '@tauri-apps/api 2.11.1' "$npm_notice_dir/notices.md"
grep -Fq 'Copyright (c) 2017 - Present Tauri Apps Contributors' \
    "$npm_notice_dir/notices.md"
grep -Fq 'react 19.2.8' "$npm_notice_dir/notices.md"
grep -Fq 'Copyright (c) Meta Platforms, Inc. and affiliates.' \
    "$npm_notice_dir/notices.md"
if grep -Fq "$project_root" "$npm_notice_dir/notices.md"; then
    printf '%s\n' 'npm notices contain the private workspace path' >&2
    exit 1
fi

if grep -Fq 'OverCrow by PlayerVox' \
        README.md \
        docs/testing/manual-mvp.md \
        packaging/arch/overcrow.install \
        packaging/applications/com.playervox.OverCrow.desktop \
        packaging/metainfo/com.playervox.OverCrow.metainfo.xml \
        crates/overcrow-control-ui/index.html \
        crates/overcrow-control-ui/src-tauri/tauri.conf.json; then
    printf '%s\n' 'legacy visible product name remains' >&2
    exit 1
fi

if grep -E -i -n \
        'proprietary software|repository is not open source|not accepting external source contributions' \
        LICENSE NOTICE TRADEMARKS.md CONTRIBUTING.md README.md Cargo.toml \
        integrations/kwin/metadata.json; then
    printf '%s\n' 'current first-party policy still describes OverCrow as proprietary' >&2
    exit 1
fi
