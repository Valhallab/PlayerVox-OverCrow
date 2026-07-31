#!/bin/sh
set -eu

usage() {
    printf '%s\n' \
        'usage: build-repository.sh VERSION SOURCE_DIR BASE_REPOSITORY OUTPUT_DIR SIGNING_FINGERPRINT' \
        >&2
    exit 2
}

test "$#" -eq 5 || usage

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
# shellcheck disable=SC1090,SC1091
. "$project_root/scripts/lib/release-version.sh"

version=$1
source_dir=$2
base_repository=$3
output_dir=$4
signing_fingerprint=$5

fail() {
    printf '%s\n' "error: $*" >&2
    exit 1
}

canonical_directory() {
    candidate=$1
    description=$2
    case $candidate in
        /*) ;;
        *) fail "$description must be absolute" ;;
    esac
    test -d "$candidate" && test ! -L "$candidate" ||
        fail "$description is not a real directory"
    canonical=$(realpath -e -- "$candidate") ||
        fail "$description cannot be canonicalized"
    test "$canonical" = "$candidate" ||
        fail "$description must already be canonical"
    printf '%s\n' "$canonical"
}

if ! overcrow_version_is_valid "$version"; then
    fail 'invalid release version'
fi
if ! deb_version=$(overcrow_deb_package_version "$version"); then
    fail 'could not normalize the Debian package version'
fi

source_dir=$(canonical_directory "$source_dir" 'SOURCE_DIR')
base_repository=$(canonical_directory "$base_repository" 'BASE_REPOSITORY')

case $output_dir in
    /*) ;;
    *) fail 'OUTPUT_DIR must be absolute' ;;
esac
if test -e "$output_dir" || test -L "$output_dir"; then
    fail 'OUTPUT_DIR already exists'
fi
output_parent=$(dirname -- "$output_dir")
output_parent=$(canonical_directory "$output_parent" 'OUTPUT_DIR parent')
test "$output_dir" = "$output_parent/$(basename -- "$output_dir")" ||
    fail 'OUTPUT_DIR must already be canonical'

if ! printf '%s\n' "$signing_fingerprint" |
        LC_ALL=C grep -Eq '^[0-9A-F]{40}$'; then
    fail 'SIGNING_FINGERPRINT must be one full uppercase OpenPGP fingerprint'
fi
case ${SOURCE_DATE_EPOCH-} in
    '' | *[!0-9]*) fail 'SOURCE_DATE_EPOCH must be a positive integer' ;;
    0) fail 'SOURCE_DATE_EPOCH must be a positive integer' ;;
esac

for program in ar awk bsdtar cmp date find gpg grep gzip install md5sum \
        mktemp realpath sha1sum sha256sum sort stat tar; do
    command -v "$program" >/dev/null 2>&1 ||
        fail "required program is unavailable: $program"
done

artifact="overcrow_${deb_version}_amd64.deb"
artifact_path="$source_dir/$artifact"
checksums="$source_dir/SHA256SUMS"
test -f "$artifact_path" && test ! -L "$artifact_path" &&
        test -s "$artifact_path" ||
    fail "invalid release artifact: $artifact"
test -f "$checksums" && test ! -L "$checksums" ||
    fail 'invalid release checksum file'

checksum_record=$(
    awk -v artifact="$artifact" '
        $2 == artifact && $1 ~ /^[0-9a-f]{64}$/ {
            print $1
            count++
        }
        END {
            if (count != 1) {
                exit 1
            }
        }
    ' "$checksums"
) || fail 'release checksum entry is missing or ambiguous'
actual_checksum=$(sha256sum "$artifact_path" | awk '{ print $1 }')
test "$checksum_record" = "$actual_checksum" ||
    fail 'release artifact checksum mismatch'

secret_listing=$(
    gpg --batch --with-colons --list-secret-keys "$signing_fingerprint" \
        2>/dev/null
) || fail 'signing key is unavailable'
test "$(
    printf '%s\n' "$secret_listing" |
        awk -F: -v fingerprint="$signing_fingerprint" '
            $1 == "fpr" && $10 == fingerprint { count++ }
            END { print count + 0 }
        '
)" -eq 1 || fail 'signing key fingerprint is ambiguous'
printf '%s\n' "$secret_listing" |
    awk -F: '$1 == "sec" && $12 ~ /s/ { found = 1 } END { exit !found }' ||
    fail 'signing key cannot sign repository metadata'

working=
verify_home=
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    if test -n "$working" &&
            { test -e "$working" || test -L "$working"; }; then
        rm -rf -- "$working"
    fi
    if test -n "$verify_home" &&
            { test -e "$verify_home" || test -L "$verify_home"; }; then
        rm -rf -- "$verify_home"
    fi
    exit "$status"
}
trap cleanup EXIT HUP INT TERM

umask 077
working=$(mktemp -d "$output_parent/.overcrow-apt-repository.XXXXXX")
verify_home=$(mktemp -d "$output_parent/.overcrow-apt-verify.XXXXXX")

pool="$working/pool/main/o/overcrow"
binary_dir="$working/dists/stable/main/binary-amd64"
keyring_dir="$working/keyrings"
mkdir -p "$pool" "$binary_dir/by-hash/SHA256" "$keyring_dir"

control_field() {
    control_file=$1
    field=$2
    awk -F': ' -v field="$field" '
        $1 == field {
            value = substr($0, length(field) + 3)
            count++
        }
        END {
            if (count != 1) {
                exit 1
            }
            print value
        }
    ' "$control_file"
}

validate_deb() {
    deb=$1
    control_output=$2
    test -f "$deb" && test ! -L "$deb" && test -s "$deb" ||
        fail 'repository contains an invalid DEB'
    deb_name=$(basename -- "$deb")
    printf '%s\n' "$deb_name" |
        LC_ALL=C grep -Eq \
            '^overcrow_[0-9A-Za-z.+~-]+_amd64\.deb$' ||
        fail 'repository contains an invalid DEB filename'

    members=$(ar t "$deb") || fail 'cannot inspect DEB archive members'
    test "$members" = "debian-binary
control.tar.xz
data.tar.xz" || fail 'DEB archive members are invalid'
    ar p "$deb" control.tar.xz |
        bsdtar -xOf - ./control > "$control_output" ||
        fail 'cannot read DEB control metadata'
    test -s "$control_output" || fail 'DEB control metadata is empty'

    package=$(control_field "$control_output" Package) ||
        fail 'DEB package field is missing or ambiguous'
    package_version=$(control_field "$control_output" Version) ||
        fail 'DEB version field is missing or ambiguous'
    architecture=$(control_field "$control_output" Architecture) ||
        fail 'DEB architecture field is missing or ambiguous'
    test "$package" = overcrow || fail 'foreign package in APT pool'
    test "$architecture" = amd64 || fail 'foreign architecture in APT pool'
    printf '%s\n' "$package_version" |
        LC_ALL=C grep -Eq '^[0-9A-Za-z.+~-]+$' ||
        fail 'invalid Debian version in APT pool'
    test "$deb_name" = "overcrow_${package_version}_amd64.deb" ||
        fail 'DEB filename does not match its control version'
    if grep -Eq '^(Filename|Size|SHA256):' "$control_output"; then
        fail 'DEB control metadata contains repository-owned fields'
    fi
}

base_pool="$base_repository/pool/main/o/overcrow"
if test -e "$base_pool" || test -L "$base_pool"; then
    test -d "$base_pool" && test ! -L "$base_pool" ||
        fail 'base repository pool is invalid'
    invalid_entry=$(
        find "$base_pool" -mindepth 1 -maxdepth 1 ! -type f -print -quit
    )
    test -z "$invalid_entry" ||
        fail 'base repository pool contains a non-file entry'
    for existing_deb in "$base_pool"/*; do
        test -e "$existing_deb" || continue
        existing_control="$working/.control-existing"
        validate_deb "$existing_deb" "$existing_control"
        install -m 0644 "$existing_deb" "$pool/$(basename -- "$existing_deb")"
    done
fi

current_control="$working/.control-current"
validate_deb "$artifact_path" "$current_control"
current_destination="$pool/$artifact"
if test -e "$current_destination" || test -L "$current_destination"; then
    if ! test -f "$current_destination" ||
            test -L "$current_destination" ||
            ! cmp -s "$artifact_path" "$current_destination"; then
        fail 'same package version has conflicting content'
    fi
else
    install -m 0644 "$artifact_path" "$current_destination"
fi

packages="$binary_dir/Packages"
: > "$packages"
for indexed_deb in "$pool"/*.deb; do
    control="$working/.control-index"
    validate_deb "$indexed_deb" "$control"
    {
        cat "$control"
        printf 'Filename: pool/main/o/overcrow/%s\n' \
            "$(basename -- "$indexed_deb")"
        printf 'Size: %s\n' "$(stat -c '%s' "$indexed_deb")"
        printf 'SHA256: %s\n\n' \
            "$(sha256sum "$indexed_deb" | awk '{ print $1 }')"
    } >> "$packages"
done
gzip -n -9 -c "$packages" > "$binary_dir/Packages.gz"

for index in "$packages" "$binary_dir/Packages.gz"; do
    index_hash=$(sha256sum "$index" | awk '{ print $1 }')
    install -m 0644 "$index" "$binary_dir/by-hash/SHA256/$index_hash"
done

release="$working/dists/stable/Release"
release_date=$(
    LC_ALL=C date -u -d "@$SOURCE_DATE_EPOCH" \
        '+%a, %d %b %Y %H:%M:%S UTC'
) || fail 'cannot render the release date'
{
    printf '%s\n' \
        'Origin: PlayerVox' \
        'Label: PlayerVox OverCrow' \
        'Suite: stable' \
        'Codename: stable' \
        "Date: $release_date" \
        'Architectures: amd64' \
        'Components: main' \
        'Description: PlayerVox OverCrow Linux packages' \
        'Acquire-By-Hash: yes' \
        'SHA256:'
    for relative_index in \
            main/binary-amd64/Packages \
            main/binary-amd64/Packages.gz; do
        index="$working/dists/stable/$relative_index"
        printf ' %s %s %s\n' \
            "$(sha256sum "$index" | awk '{ print $1 }')" \
            "$(stat -c '%s' "$index")" \
            "$relative_index"
    done
} > "$release"

public_key="$keyring_dir/playervox-overcrow-archive-keyring.gpg"
gpg --batch --export-options export-minimal \
    --export "$signing_fingerprint" > "$public_key" ||
    fail 'cannot export the archive public key'
test -s "$public_key" || fail 'archive public key export is empty'

install -m 0644 "$project_root/packaging/apt/playervox-overcrow.sources" \
    "$working/playervox-overcrow.sources"
: > "$working/.nojekyll"
printf '%s\n' 1 > "$working/.overcrow-generated-apt-repository"

gpg --batch --yes --local-user "$signing_fingerprint" \
    --digest-algo SHA256 --clearsign \
    --output "$working/dists/stable/InRelease" "$release" ||
    fail 'cannot create InRelease signature'
gpg --batch --yes --local-user "$signing_fingerprint" \
    --digest-algo SHA256 --detach-sign \
    --output "$working/dists/stable/Release.gpg" "$release" ||
    fail 'cannot create detached Release signature'

chmod 0755 "$working" "$working/dists" "$working/dists/stable" \
    "$working/dists/stable/main" "$binary_dir" \
    "$binary_dir/by-hash" "$binary_dir/by-hash/SHA256" \
    "$working/keyrings" "$working/pool" "$working/pool/main" \
    "$working/pool/main/o" "$pool"
find "$working" -type f -exec chmod 0644 {} +

GNUPGHOME="$verify_home" gpg --batch --import "$public_key" \
    >/dev/null 2>&1 || fail 'cannot import the public verification key'
verified_fingerprint=$(
    GNUPGHOME="$verify_home" gpg --batch --with-colons --fingerprint \
        2>/dev/null |
        awk -F: '$1 == "fpr" { print $10 }'
)
test "$verified_fingerprint" = "$signing_fingerprint" ||
    fail 'exported public key fingerprint mismatch'
if GNUPGHOME="$verify_home" gpg --batch --with-colons \
        --list-secret-keys 2>/dev/null | grep -q '^sec:'; then
    fail 'public repository contains secret key material'
fi
GNUPGHOME="$verify_home" gpg --batch --verify \
    "$working/dists/stable/InRelease" >/dev/null 2>&1 ||
    fail 'InRelease signature verification failed'
GNUPGHOME="$verify_home" gpg --batch --verify \
    "$working/dists/stable/Release.gpg" "$release" >/dev/null 2>&1 ||
    fail 'Release signature verification failed'

for relative_index in \
        main/binary-amd64/Packages \
        main/binary-amd64/Packages.gz; do
    index="$working/dists/stable/$relative_index"
    index_hash=$(sha256sum "$index" | awk '{ print $1 }')
    cmp -s "$index" \
        "$binary_dir/by-hash/SHA256/$index_hash" ||
        fail 'by-hash index verification failed'
done

if test -e "$output_dir" || test -L "$output_dir"; then
    fail 'OUTPUT_DIR appeared during repository generation'
fi
mv -T -n -- "$working" "$output_dir" ||
    fail 'cannot publish the generated repository'
working=

printf '%s\n' "APT repository candidate ready: $output_dir"
printf '%s\n' 'Nothing was installed, committed, pushed, or started.'
