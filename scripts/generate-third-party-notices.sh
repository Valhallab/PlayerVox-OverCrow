#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:?usage: generate-third-party-notices.sh OUTPUT}

case $output in
    /*) ;;
    *) printf '%s\n' 'error: OUTPUT must be an absolute path' >&2; exit 2 ;;
esac

output_dir=$(dirname -- "$output")
test -d "$output_dir" || {
    printf '%s\n' "error: output directory does not exist: $output_dir" >&2
    exit 2
}

about_version=$(cargo about --version 2>/dev/null) || {
    printf '%s\n' 'error: cargo-about 0.9.1 is required' >&2
    exit 1
}
test "$about_version" = 'cargo-about 0.9.1' || {
    printf '%s\n' 'error: cargo-about 0.9.1 is required' >&2
    exit 1
}

tmp=$(mktemp "$output_dir/.overcrow-third-party.XXXXXX")
trap 'rm -f -- "$tmp"' EXIT HUP INT TERM

cd "$project_root"
cargo about generate --locked --offline --fail --workspace \
    --config about.toml \
    --output-file "$tmp" \
    packaging/licenses/third-party.hbs
test -s "$tmp"
chmod 0644 "$tmp"
mv -f -- "$tmp" "$output"
trap - EXIT HUP INT TERM
