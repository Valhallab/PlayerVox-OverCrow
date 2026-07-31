#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
builder="$project_root/packaging/apt/build-repository.sh"
source_definition="$project_root/packaging/apt/playervox-overcrow.sources"

test -x "$builder"
test -f "$source_definition"

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/overcrow-apt-smoke.XXXXXX")
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    rm -rf -- "$work_dir"
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

create_deb() {
    output=$1
    package=$2
    version=$3
    architecture=$4
    payload=$5
    fixture="$work_dir/deb-fixture-$(basename -- "$output")-$payload"

    mkdir -p "$fixture/control" "$fixture/data/usr/share/overcrow"
    {
        printf 'Package: %s\n' "$package"
        printf 'Version: %s\n' "$version"
        printf 'Architecture: %s\n' "$architecture"
        printf '%s\n' 'Maintainer: Valhallab <contact@valhallab.com>'
        printf '%s\n' 'Description: PlayerVox OverCrow smoke fixture'
    } > "$fixture/control/control"
    printf '%s\n' "$payload" > "$fixture/data/usr/share/overcrow/payload"
    printf '2.0\n' > "$fixture/debian-binary"
    tar -C "$fixture/control" -cJf "$fixture/control.tar.xz" ./control
    tar -C "$fixture/data" -cJf "$fixture/data.tar.xz" .
    ar crD "$output" "$fixture/debian-binary" "$fixture/control.tar.xz" \
        "$fixture/data.tar.xz" >/dev/null
}

write_checksums() {
    directory=$1
    (
        cd "$directory"
        sha256sum -- *.deb > SHA256SUMS
    )
}

expect_failure() {
    description=$1
    shift
    if "$@" >/dev/null 2>&1; then
        printf '%s\n' "APT repository builder accepted $description" >&2
        exit 1
    fi
}

release_dir="$work_dir/release"
empty_base="$work_dir/empty-base"
repository="$work_dir/repository"
key_home="$work_dir/key-home"
mkdir -m 0700 "$release_dir" "$empty_base" "$key_home"

deb_name='overcrow_0.1.0~pre.alpha.5-1_amd64.deb'
deb="$release_dir/$deb_name"
create_deb "$deb" overcrow '0.1.0~pre.alpha.5-1' amd64 current
write_checksums "$release_dir"

GNUPGHOME="$key_home" gpg --batch --passphrase '' --quick-generate-key \
    'OverCrow APT smoke <smoke@example.invalid>' rsa2048 sign 1d \
    >/dev/null 2>&1
fingerprint=$(
    GNUPGHOME="$key_home" gpg --batch --with-colons --fingerprint \
        'OverCrow APT smoke <smoke@example.invalid>' 2>/dev/null |
        awk -F: '$1 == "fpr" { print $10; exit }'
)
test -n "$fingerprint"

GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448840 \
    "$builder" 0.1.0-pre-alpha.5 "$release_dir" "$empty_base" \
    "$repository" "$fingerprint"

test -f "$repository/.nojekyll"
test -f "$repository/.overcrow-generated-apt-repository"
test -f "$repository/dists/stable/InRelease"
test -f "$repository/dists/stable/Release"
test -f "$repository/dists/stable/Release.gpg"
test -f "$repository/dists/stable/main/binary-amd64/Packages"
test -f "$repository/dists/stable/main/binary-amd64/Packages.gz"
test -f \
    "$repository/keyrings/playervox-overcrow-archive-keyring.gpg"
cmp "$source_definition" "$repository/playervox-overcrow.sources"
cmp "$deb" "$repository/pool/main/o/overcrow/$deb_name"

packages="$repository/dists/stable/main/binary-amd64/Packages"
for expected in \
        'Package: overcrow' \
        'Version: 0.1.0~pre.alpha.5-1' \
        'Architecture: amd64' \
        "Filename: pool/main/o/overcrow/$deb_name" \
        "Size: $(stat -c '%s' "$deb")" \
        "SHA256: $(sha256sum "$deb" | awk '{ print $1 }')"; do
    test "$(grep -Fxc "$expected" "$packages")" -eq 1
done

release="$repository/dists/stable/Release"
for expected in \
        'Origin: PlayerVox' \
        'Label: PlayerVox OverCrow' \
        'Suite: stable' \
        'Codename: stable' \
        'Architectures: amd64' \
        'Components: main' \
        'Acquire-By-Hash: yes'; do
    test "$(grep -Fxc "$expected" "$release")" -eq 1
done

for index in Packages Packages.gz; do
    index_path="$repository/dists/stable/main/binary-amd64/$index"
    index_hash=$(sha256sum "$index_path" | awk '{ print $1 }')
    by_hash="$repository/dists/stable/main/binary-amd64/by-hash/SHA256/$index_hash"
    test -f "$by_hash"
    cmp "$index_path" "$by_hash"
    grep -Fq \
        "$index_hash $(stat -c '%s' "$index_path") main/binary-amd64/$index" \
        "$release"
done

verify_home="$work_dir/verify-home"
mkdir -m 0700 "$verify_home"
GNUPGHOME="$verify_home" gpg --batch --import \
    "$repository/keyrings/playervox-overcrow-archive-keyring.gpg" \
    >/dev/null 2>&1
GNUPGHOME="$verify_home" gpg --batch --verify \
    "$repository/dists/stable/InRelease" >/dev/null 2>&1
GNUPGHOME="$verify_home" gpg --batch --verify \
    "$repository/dists/stable/Release.gpg" \
    "$repository/dists/stable/Release" >/dev/null 2>&1
if GNUPGHOME="$verify_home" gpg --batch --with-colons \
        --list-secret-keys 2>/dev/null | grep -q '^sec:'; then
    printf '%s\n' 'public APT repository exported secret key material' >&2
    exit 1
fi

bad_checksum="$work_dir/bad-checksum"
mkdir "$bad_checksum"
cp "$deb" "$bad_checksum/$deb_name"
printf '%064d  %s\n' 0 "$deb_name" > "$bad_checksum/SHA256SUMS"
expect_failure 'a bad release checksum' \
    env GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448840 \
    "$builder" 0.1.0-pre-alpha.5 "$bad_checksum" "$empty_base" \
    "$work_dir/bad-checksum-output" "$fingerprint"

wrong_package="$work_dir/wrong-package"
mkdir "$wrong_package"
create_deb "$wrong_package/$deb_name" attacker \
    '0.1.0~pre.alpha.5-1' amd64 wrong-package
write_checksums "$wrong_package"
expect_failure 'a foreign package' \
    env GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448840 \
    "$builder" 0.1.0-pre-alpha.5 "$wrong_package" "$empty_base" \
    "$work_dir/wrong-package-output" "$fingerprint"

wrong_arch="$work_dir/wrong-arch"
mkdir "$wrong_arch"
create_deb "$wrong_arch/$deb_name" overcrow \
    '0.1.0~pre.alpha.5-1' arm64 wrong-architecture
write_checksums "$wrong_arch"
expect_failure 'a foreign architecture' \
    env GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448840 \
    "$builder" 0.1.0-pre-alpha.5 "$wrong_arch" "$empty_base" \
    "$work_dir/wrong-arch-output" "$fingerprint"

symlink_release="$work_dir/symlink-release"
mkdir "$symlink_release"
ln -s "$deb" "$symlink_release/$deb_name"
write_checksums "$symlink_release"
expect_failure 'a symlinked package' \
    env GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448840 \
    "$builder" 0.1.0-pre-alpha.5 "$symlink_release" "$empty_base" \
    "$work_dir/symlink-output" "$fingerprint"

conflicting_base="$work_dir/conflicting-base"
mkdir -p "$conflicting_base/pool/main/o/overcrow"
create_deb "$conflicting_base/pool/main/o/overcrow/$deb_name" overcrow \
    '0.1.0~pre.alpha.5-1' amd64 conflicting
expect_failure 'conflicting content for one version' \
    env GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448840 \
    "$builder" 0.1.0-pre-alpha.5 "$release_dir" "$conflicting_base" \
    "$work_dir/conflicting-output" "$fingerprint"

existing_output="$work_dir/existing-output"
mkdir "$existing_output"
expect_failure 'an existing output directory' \
    env GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448840 \
    "$builder" 0.1.0-pre-alpha.5 "$release_dir" "$empty_base" \
    "$existing_output" "$fingerprint"

expect_failure 'a relative source path' \
    env GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448840 \
    "$builder" 0.1.0-pre-alpha.5 relative "$empty_base" \
    "$work_dir/relative-output" "$fingerprint"

expect_failure 'an unknown signing fingerprint' \
    env GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448840 \
    "$builder" 0.1.0-pre-alpha.5 "$release_dir" "$empty_base" \
    "$work_dir/unknown-key-output" \
    '0000000000000000000000000000000000000000'

publisher="$project_root/scripts/publish-apt-repository.sh"
test -x "$publisher"

old_release="$work_dir/old-release"
old_repository="$work_dir/old-repository"
mkdir "$old_release"
old_deb_name='overcrow_0.1.0~pre.alpha.4-1_amd64.deb'
create_deb "$old_release/$old_deb_name" overcrow \
    '0.1.0~pre.alpha.4-1' amd64 old
write_checksums "$old_release"
GNUPGHOME="$key_home" SOURCE_DATE_EPOCH=1785448800 \
    "$builder" 0.1.0-pre-alpha.4 "$old_release" "$empty_base" \
    "$old_repository" "$fingerprint"

remote="$work_dir/apt-remote.git"
seed="$work_dir/apt-seed"
git init --bare "$remote" >/dev/null
cp -a "$old_repository" "$seed"
git -C "$seed" init -b gh-pages >/dev/null
git -C "$seed" config user.name 'Valhallab'
git -C "$seed" config user.email 'contact@valhallab.com'
git -C "$seed" add .
git -C "$seed" commit -m 'Initial APT repository' >/dev/null
git -C "$seed" remote add origin "$remote"
git -C "$seed" push origin gh-pages >/dev/null
original_remote_commit=$(
    git --git-dir="$remote" rev-parse refs/heads/gh-pages
)

published_candidate="$work_dir/published-candidate"
OVERCROW_APT_TEST_MODE=1 \
OVERCROW_APT_REMOTE_URL="$remote" \
OVERCROW_APT_RELEASE_DIR="$release_dir" \
OVERCROW_APT_OUTPUT_DIR="$published_candidate" \
GNUPGHOME="$key_home" \
SOURCE_DATE_EPOCH=1785448840 \
    "$publisher" 0.1.0-pre-alpha.5 "$fingerprint"
test -d "$published_candidate"
test -f "$published_candidate/pool/main/o/overcrow/$old_deb_name"
test -f "$published_candidate/pool/main/o/overcrow/$deb_name"
test "$(
    git --git-dir="$remote" rev-parse refs/heads/gh-pages
)" = "$original_remote_commit"

OVERCROW_APT_TEST_MODE=1 \
OVERCROW_APT_REMOTE_URL="$remote" \
OVERCROW_APT_RELEASE_DIR="$release_dir" \
OVERCROW_APT_OUTPUT_DIR="$published_candidate" \
GNUPGHOME="$key_home" \
SOURCE_DATE_EPOCH=1785448840 \
    "$publisher" 0.1.0-pre-alpha.5 "$fingerprint" --push
test "$(
    git --git-dir="$remote" rev-parse refs/heads/gh-pages
)" != "$original_remote_commit"

published_checkout="$work_dir/published-checkout"
git clone --branch gh-pages "$remote" "$published_checkout" >/dev/null 2>&1
test -f "$published_checkout/pool/main/o/overcrow/$old_deb_name"
test -f "$published_checkout/pool/main/o/overcrow/$deb_name"
cmp "$published_candidate/dists/stable/InRelease" \
    "$published_checkout/dists/stable/InRelease"

if grep -Eq 'sudo|apt-key|systemctl|eval' "$builder" "$publisher" ||
        grep -Eq '[[:space:]]git[[:space:]]+push' "$builder"; then
    printf '%s\n' 'APT repository builder contains forbidden mutations' >&2
    exit 1
fi
grep -Fq -- '--push' "$publisher"
grep -Fq 'GIT_TERMINAL_PROMPT=0' "$publisher"

printf '%s\n' 'APT repository smoke test passed'
