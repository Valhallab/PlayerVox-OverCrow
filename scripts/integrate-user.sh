#!/bin/sh
# shellcheck disable=SC1090,SC2016
set -eu

installed_helper='/usr/lib/overcrow/overcrow-integrate'
installed_library='/usr/lib/overcrow/hyprland-config.sh'
installed_share='/usr/share/overcrow/integrations'
installed_owner=0
kwin_id='io.github.overcrow.kwin'
system_program_parent='/usr/bin'
system_program_owner=0
id_program='/usr/bin/id'
timeout_program='/usr/bin/timeout'
sha256_program='/usr/bin/sha256sum'
readlink_program='/usr/bin/readlink'
stat_program='/usr/bin/stat'
find_program='/usr/bin/find'
mountpoint_program='/usr/bin/mountpoint'
tr_program='/usr/bin/tr'
awk_program='/usr/bin/gawk'
basename_program='/usr/bin/basename'
dirname_program='/usr/bin/dirname'
cp_program='/usr/bin/cp'
install_program='/usr/bin/install'
mv_program='/usr/bin/mv'
mkdir_program='/usr/bin/mkdir'
flock_program='/usr/bin/flock'
sync_program='/usr/bin/sync'
rm_program='/usr/bin/rm'
rmdir_program='/usr/bin/rmdir'
kpackagetool_program='/usr/bin/kpackagetool6'
kreadconfig_program='/usr/bin/kreadconfig6'
kwriteconfig_program='/usr/bin/kwriteconfig6'
qdbus_program='/usr/bin/qdbus6'
# Every supported KWin release has an exact four-entry tree and one reviewed
# metadata/main fingerprint pair. This history is append-only: new releases
# add a current pair and retain every supported reviewed legacy pair. Never add
# a fingerprint from an untrusted installed package.
kwin_current_metadata_sha256='72844f4e860c98974fa240a4fb1620d0ea25db6cd9facfe46dde3dbdb9adeb70'
kwin_current_main_sha256='9fc7a92d1f2936e454ac83bc7b187110b7d22fae5f93bd355dd99557e656259d'
kwin_legacy_pre_alpha_1_metadata_sha256='d3f2a92714dbd0fb2c497341d9ae7eabd5498e7c87047a77dd7dcf9c54889f83'
kwin_legacy_pre_alpha_1_main_sha256='9fc7a92d1f2936e454ac83bc7b187110b7d22fae5f93bd355dd99557e656259d'
kwin_legacy_0_1_0_metadata_sha256='90526be045929587ff25ba1d028e07201925e80dd8bcac72bc5a1ca6297be670'
kwin_legacy_0_1_0_main_sha256='9fc7a92d1f2936e454ac83bc7b187110b7d22fae5f93bd355dd99557e656259d'
kwin_legacy_task7_metadata_sha256='4fdee6317eef034d5b23de338e9b2ee6797144ea15de58e02ab9b2417d9b2c8a'
kwin_legacy_task7_main_sha256='9db8a46fcffeffb4f66b8ddcaee7874da5ca22215fe64bc976c634c2b7c81036'
kwin_legacy_mvp_metadata_sha256='c61c753e3887510a4b1d3659ca84aef281072bfa87a82a85ed84e89af08aa666'
kwin_legacy_mvp_main_sha256='9db8a46fcffeffb4f66b8ddcaee7874da5ca22215fe64bc976c634c2b7c81036'

fail() {
    printf '%s\n' "error: $1" >&2
    exit 1
}

trusted_executable() {
    overcrow_program=$1
    [ "${overcrow_program%/*}" = "$system_program_parent" ] || return 1
    [ -f "$overcrow_program" ] && [ ! -L "$overcrow_program" ] && \
        [ -x "$overcrow_program" ] || return 1
    overcrow_program_real=$("$readlink_program" -f -- "$overcrow_program" 2>/dev/null) || return 1
    [ "$overcrow_program_real" = "$overcrow_program" ] || return 1
    overcrow_program_permissions=$("$stat_program" -c '%A' -- "$overcrow_program" 2>/dev/null) || \
        return 1
    overcrow_program_owner=$("$stat_program" -c '%u' -- "$overcrow_program" 2>/dev/null) || \
        return 1
    [ "$overcrow_program_owner" = "$system_program_owner" ] || return 1
    case $overcrow_program_permissions in
        ?????w????|????????w?) return 1 ;;
    esac
}

trusted_system_program_parent_ready() {
    [ -d "$system_program_parent" ] && [ ! -L "$system_program_parent" ] || return 1
    overcrow_program_parent_real=$(
        "$readlink_program" -f -- "$system_program_parent" 2>/dev/null
    ) || return 1
    [ "$overcrow_program_parent_real" = "$system_program_parent" ] || return 1
    overcrow_program_parent_owner=$(
        "$stat_program" -c '%u' -- "$system_program_parent" 2>/dev/null
    ) || return 1
    [ "$overcrow_program_parent_owner" = "$system_program_owner" ] || return 1
    overcrow_program_parent_permissions=$(
        "$stat_program" -c '%A' -- "$system_program_parent" 2>/dev/null
    ) || return 1
    case $overcrow_program_parent_permissions in
        ?????w????|????????w?) return 1 ;;
    esac
}

trusted_bootstrap_programs_ready() {
    trusted_system_program_parent_ready || return 1
    for overcrow_system_program in \
        "$id_program" \
        "$readlink_program" \
        "$stat_program" \
        "$basename_program" \
        "$dirname_program"; do
        trusted_executable "$overcrow_system_program" || return 1
    done
}

trusted_resource_programs_ready() {
    for overcrow_system_program in \
        "$sha256_program" \
        "$timeout_program" \
        "$find_program" \
        "$mountpoint_program" \
        "$tr_program" \
        "$awk_program" \
        "$cp_program" \
        "$install_program" \
        "$mv_program" \
        "$mkdir_program" \
        "$flock_program" \
        "$sync_program" \
        "$rm_program" \
        "$rmdir_program"; do
        trusted_executable "$overcrow_system_program" || return 1
    done
}

trusted_plasma_programs_ready() {
    for overcrow_system_program in \
        "$kpackagetool_program" \
        "$kreadconfig_program" \
        "$kwriteconfig_program"; do
        trusted_executable "$overcrow_system_program" || return 1
    done
    if [ -e "$qdbus_program" ] || [ -L "$qdbus_program" ]; then
        trusted_executable "$qdbus_program" || return 1
        qdbus_ready=true
    else
        qdbus_ready=false
    fi
}

file_sha256() {
    overcrow_digest_file=$1
    trusted_executable "$sha256_program" || return 1
    overcrow_digest_output=$("$sha256_program" "$overcrow_digest_file" 2>/dev/null) || return 1
    overcrow_actual_digest=${overcrow_digest_output%% *}
    case $overcrow_actual_digest in
        *[!0-9a-f]*|'') return 1 ;;
    esac
    [ "${#overcrow_actual_digest}" -eq 64 ] || return 1
    printf '%s\n' "$overcrow_actual_digest"
}

file_matches_sha256() {
    overcrow_digest_file=$1
    overcrow_expected_digest=$2
    overcrow_actual_digest=$(file_sha256 "$overcrow_digest_file") || return 1
    [ "$overcrow_actual_digest" = "$overcrow_expected_digest" ]
}

trusted_bootstrap_programs_ready || fail 'trusted bootstrap programs are unavailable or unsafe'
current_uid=$("$id_program" -u 2>/dev/null) || fail 'unable to determine the current UID'
case $current_uid in
    ''|*[!0-9]*) fail 'invalid current UID' ;;
esac
[ "$current_uid" -ne 0 ] || fail 'refusing to integrate OverCrow as root'

[ "$#" -eq 1 ] || fail 'usage: overcrow-integrate enable|status'
action=$1
case $action in
    enable|status) ;;
    *) fail 'usage: overcrow-integrate enable|status' ;;
esac
system_program_error=
trusted_resource_programs_ready || \
    system_program_error='trusted system programs are unavailable or unsafe'

safe_path() {
    case $1 in
        /*) ;;
        *) return 1 ;;
    esac
    case $1 in
        *[!A-Za-z0-9_./-]*) return 1 ;;
    esac
}

direct_directory() {
    overcrow_directory=$1
    overcrow_expected_owner=$2
    safe_path "$overcrow_directory" || return 1
    [ -d "$overcrow_directory" ] && [ ! -L "$overcrow_directory" ] || return 1
    overcrow_real=$("$readlink_program" -f -- "$overcrow_directory" 2>/dev/null) || return 1
    [ "$overcrow_real" = "$overcrow_directory" ] || return 1
    overcrow_owner=$("$stat_program" -c '%u' -- "$overcrow_directory" 2>/dev/null) || return 1
    [ "$overcrow_owner" = "$overcrow_expected_owner" ] || return 1
    overcrow_permissions=$("$stat_program" -c '%A' -- "$overcrow_directory" 2>/dev/null) || return 1
    case $overcrow_permissions in
        ?????w????|????????w?) return 1 ;;
    esac
}

regular_file() {
    overcrow_file=$1
    overcrow_expected_owner=$2
    safe_path "$overcrow_file" || return 1
    [ -f "$overcrow_file" ] && [ ! -L "$overcrow_file" ] || return 1
    overcrow_real=$("$readlink_program" -f -- "$overcrow_file" 2>/dev/null) || return 1
    [ "$overcrow_real" = "$overcrow_file" ] || return 1
    overcrow_owner=$("$stat_program" -c '%u' -- "$overcrow_file" 2>/dev/null) || return 1
    [ "$overcrow_owner" = "$overcrow_expected_owner" ] || return 1
    overcrow_permissions=$("$stat_program" -c '%A' -- "$overcrow_file" 2>/dev/null) || return 1
    case $overcrow_permissions in
        ?????w????|????????w?) return 1 ;;
    esac
}

path_device() {
    overcrow_device=$(
        "$stat_program" -c '%d' -- "$1" 2>/dev/null
    ) || return 1
    case $overcrow_device in
        ''|*[!0-9]*) return 1 ;;
    esac
    printf '%s\n' "$overcrow_device"
}

path_not_mountpoint() {
    if "$mountpoint_program" -q -- "$1" 2>/dev/null; then
        return 1
    else
        overcrow_mountpoint_status=$?
    fi
    case $overcrow_mountpoint_status in
        1|32) return 0 ;;
        *) return 1 ;;
    esac
}

kwin_tree_device_and_mount_safe() {
    overcrow_tree_root=$1
    overcrow_tree_parent=${overcrow_tree_root%/*}
    overcrow_parent_device=$(path_device "$overcrow_tree_parent") || return 1
    for overcrow_tree_entry in \
        "$overcrow_tree_root" \
        "$overcrow_tree_root/contents" \
        "$overcrow_tree_root/contents/code" \
        "$overcrow_tree_root/metadata.json" \
        "$overcrow_tree_root/contents/code/main.js"; do
        overcrow_entry_device=$(path_device "$overcrow_tree_entry") || return 1
        [ "$overcrow_entry_device" = "$overcrow_parent_device" ] || return 1
    done
    path_not_mountpoint "$overcrow_tree_parent" || return 1
    overcrow_mounted_directory=$("$find_program" -P "$overcrow_tree_root" \
        -exec "$mountpoint_program" -q -- '{}' \; \
        -print -quit 2>/dev/null) || return 1
    [ -z "$overcrow_mounted_directory" ]
}

kwin_tree_exact() {
    overcrow_tree_root=$1
    overcrow_tree_owner=$2
    direct_directory "$overcrow_tree_root" "$overcrow_tree_owner" || return 1
    direct_directory "$overcrow_tree_root/contents" "$overcrow_tree_owner" || return 1
    direct_directory "$overcrow_tree_root/contents/code" "$overcrow_tree_owner" || return 1
    regular_file "$overcrow_tree_root/metadata.json" "$overcrow_tree_owner" || return 1
    regular_file "$overcrow_tree_root/contents/code/main.js" "$overcrow_tree_owner" || return 1
    kwin_tree_device_and_mount_safe "$overcrow_tree_root" || return 1
    overcrow_unexpected_entry=$("$find_program" -P "$overcrow_tree_root" \
        -mindepth 1 \
        ! \( \
            -path "$overcrow_tree_root/contents" -o \
            -path "$overcrow_tree_root/contents/code" -o \
            -path "$overcrow_tree_root/contents/code/main.js" -o \
            -path "$overcrow_tree_root/metadata.json" \
        \) -print -quit 2>/dev/null) || return 1
    [ -z "$overcrow_unexpected_entry" ]
}

kwin_cleanup_tree_safe() {
    overcrow_cleanup_root=$1
    overcrow_cleanup_owner=$2
    overcrow_cleanup_parent=${overcrow_cleanup_root%/*}
    direct_directory "$overcrow_cleanup_parent" "$overcrow_cleanup_owner" || return 1
    direct_directory "$overcrow_cleanup_root" "$overcrow_cleanup_owner" || return 1
    overcrow_cleanup_device=$(path_device "$overcrow_cleanup_parent") || return 1
    for overcrow_cleanup_entry in \
        "$overcrow_cleanup_root" \
        "$overcrow_cleanup_root/contents" \
        "$overcrow_cleanup_root/contents/code" \
        "$overcrow_cleanup_root/metadata.json" \
        "$overcrow_cleanup_root/contents/code/main.js"; do
        if [ -e "$overcrow_cleanup_entry" ] && [ ! -L "$overcrow_cleanup_entry" ]; then
            overcrow_entry_device=$(path_device "$overcrow_cleanup_entry") || return 1
            [ "$overcrow_entry_device" = "$overcrow_cleanup_device" ] || return 1
        fi
    done
    path_not_mountpoint "$overcrow_cleanup_parent" || return 1
    overcrow_mounted_directory=$("$find_program" -P "$overcrow_cleanup_root" \
        -exec "$mountpoint_program" -q -- '{}' \; \
        -print -quit 2>/dev/null) || return 1
    [ -z "$overcrow_mounted_directory" ]
}

kwin_package_version() {
    overcrow_package_root=$1
    overcrow_package_owner=$2
    kwin_tree_exact "$overcrow_package_root" "$overcrow_package_owner" || return 1
    overcrow_metadata_digest=$(file_sha256 "$overcrow_package_root/metadata.json") || return 1
    overcrow_main_digest=$(file_sha256 \
        "$overcrow_package_root/contents/code/main.js") || return 1
    case "$overcrow_metadata_digest:$overcrow_main_digest" in
        "$kwin_current_metadata_sha256:$kwin_current_main_sha256")
            printf '%s\n' current
            ;;
        "$kwin_legacy_pre_alpha_1_metadata_sha256:$kwin_legacy_pre_alpha_1_main_sha256")
            printf '%s\n' legacy-pre-alpha-1
            ;;
        "$kwin_legacy_0_1_0_metadata_sha256:$kwin_legacy_0_1_0_main_sha256")
            printf '%s\n' legacy-0.1.0
            ;;
        "$kwin_legacy_task7_metadata_sha256:$kwin_legacy_task7_main_sha256")
            printf '%s\n' legacy-task7
            ;;
        "$kwin_legacy_mvp_metadata_sha256:$kwin_legacy_mvp_main_sha256")
            printf '%s\n' legacy-mvp
            ;;
        *) return 1 ;;
    esac
}

script_directory=$(CDPATH='' cd -P -- "$("$dirname_program" -- "$0")" 2>/dev/null && pwd) || \
    fail 'unable to resolve integration helper directory'
script_path="$script_directory/$("$basename_program" -- "$0")"
if [ -L "$0" ] || [ -L "$script_path" ]; then
    fail 'refusing a symlinked integration helper'
fi
script_real=$("$readlink_program" -f -- "$script_path" 2>/dev/null) || \
    fail 'unable to resolve integration helper'
[ "$script_real" = "$script_path" ] || fail 'refusing an unsafe integration helper path'

resource_error=$system_program_error
[ "$script_path" = "$installed_helper" ] || \
    fail 'refusing an unknown integration resource layout'
library=$installed_library
integration_root=$installed_share
resource_owner=$installed_owner

hyprland_templates="$integration_root/hyprland"
kwin_package="$integration_root/kwin"
if [ -z "$resource_error" ]; then
    installed_library_parent=$("$dirname_program" -- "$installed_library")
    installed_share_parent=$("$dirname_program" -- "$installed_share")
    if ! direct_directory "$installed_library_parent" "$resource_owner" || \
        ! direct_directory "$installed_share_parent" "$resource_owner" || \
        ! direct_directory "$installed_share" "$resource_owner"; then
        resource_error='integration resource parents are missing or unsafe'
    fi
fi

if [ -z "$resource_error" ] && { \
    ! regular_file "$script_path" "$resource_owner" || \
    ! regular_file "$library" "$resource_owner" || \
    ! direct_directory "$hyprland_templates" "$resource_owner" || \
    ! regular_file "$hyprland_templates/overcrow.conf.in" "$resource_owner" || \
    ! regular_file "$hyprland_templates/overcrow.lua.in" "$resource_owner" || \
    ! direct_directory "$kwin_package" "$resource_owner" || \
    ! direct_directory "$kwin_package/contents" "$resource_owner" || \
    ! direct_directory "$kwin_package/contents/code" "$resource_owner" || \
    ! regular_file "$kwin_package/metadata.json" "$resource_owner" || \
    ! regular_file "$kwin_package/contents/code/main.js" "$resource_owner"; \
}; then
    resource_error='integration resources are missing or unsafe'
fi
if [ -z "$resource_error" ]; then
    overcrow_source_package_version=$(kwin_package_version \
        "$kwin_package" "$resource_owner") || overcrow_source_package_version=
    if [ "$overcrow_source_package_version" != current ]; then
        resource_error='integration package identity is not the pinned OverCrow package'
    fi
fi

normalize_hint() {
    overcrow_hint=$1
    [ "${#overcrow_hint}" -le 64 ] || {
        printf '%s\n' other
        return
    }
    case $overcrow_hint in
        *[!A-Za-z0-9_-]*)
            printf '%s\n' other
            return
            ;;
    esac
    overcrow_hint=$(printf '%s' "$overcrow_hint" | \
        "$tr_program" '[:upper:]' '[:lower:]')
    case $overcrow_hint in
        '') printf '%s\n' empty ;;
        hyprland) printf '%s\n' hyprland ;;
        kde|plasma) printf '%s\n' plasma ;;
        *) printf '%s\n' other ;;
    esac
}

current_desktop=$(normalize_hint "${XDG_CURRENT_DESKTOP:-}")
desktop_session=$(normalize_hint "${DESKTOP_SESSION:-}")
desktop=unsupported
desktop_error='unsupported desktop session'
case "$current_desktop:$desktop_session" in
    hyprland:plasma|plasma:hyprland)
        desktop=ambiguous
        desktop_error='ambiguous desktop session'
        ;;
    hyprland:hyprland|hyprland:empty|hyprland:other|empty:hyprland)
        desktop=hyprland
        desktop_error=
        ;;
    plasma:plasma|plasma:empty|plasma:other|empty:plasma)
        desktop=plasma
        desktop_error=
        ;;
esac

json_escape() {
    printf '%s' "$1" | "$awk_program" '
        BEGIN { ORS = "" }
        {
            if (NR > 1) printf "\\n"
            gsub(/\\/, "\\\\")
            gsub(/"/, "\\\"")
            gsub(/\r/, "\\r")
            gsub(/\t/, "\\t")
            printf "%s", $0
        }
    '
}

status_result() {
    overcrow_ready=$1
    overcrow_error=${2:-}
    overcrow_desktop=$(json_escape "$desktop")
    if [ "$overcrow_ready" = true ]; then
        printf '{"desktop":"%s","ready":true,"error":null}\n' "$overcrow_desktop"
    else
        overcrow_error=$(json_escape "$overcrow_error")
        printf '{"desktop":"%s","ready":false,"error":"%s"}\n' \
            "$overcrow_desktop" "$overcrow_error"
    fi
}

if [ -n "$resource_error" ]; then
    if [ "$action" = status ]; then
        status_result false "$resource_error"
        exit 0
    fi
    fail "$resource_error"
fi

run_bounded() {
    "$timeout_program" --signal=TERM --kill-after=1s 2s "$@"
}

bounded_status_is_fatal() {
    [ "$1" -eq 125 ]
}

fail_bounded_status() {
    printf '%s\n' \
        "error: bounded command could not be started during $1; durable transaction retained" \
        >&2
    exit 125
}

# This path was selected exclusively by the trusted source/installed layout above.
. "$library"
# Consumed by functions from the sourced Hyprland library.
# shellcheck disable=SC2034
overcrow_hypr_timeout_program=$timeout_program

config_home=${XDG_CONFIG_HOME:-${HOME:-}/.config}
data_home=${XDG_DATA_HOME:-${HOME:-}/.local/share}
kwin_destination="$data_home/kwin/scripts/$kwin_id"
kwin_scripts_directory="$data_home/kwin/scripts"
kwin_restore_build="$kwin_scripts_directory/.$kwin_id.overcrow-restore.build"
kwin_restore_ready="$kwin_scripts_directory/.$kwin_id.overcrow-restore.ready"
kwin_discard="$kwin_scripts_directory/.$kwin_id.overcrow-discard"
kwin_transaction_dir="$data_home/.$kwin_id.overcrow-transaction"
kwin_transaction_build="$data_home/.$kwin_id.overcrow-transaction.build"
kwin_transaction_cleanup_dir="$data_home/.$kwin_id.overcrow-transaction.cleanup"
kwin_lock_path="$data_home/.$kwin_id.overcrow.lock"

# The invoking UID already controls its compositor configuration and remains in
# the trust boundary. These checks close accidental and detectable pathname
# races; they do not claim to defeat a continuously malicious same-UID process.
kwin_open_lock_matches_path() {
    overcrow_lock_fd_path="/proc/$$/fd/9"
    [ -f "$overcrow_lock_fd_path" ] || return 1
    overcrow_fd_owner=$("$stat_program" -L -c '%u' -- \
        "$overcrow_lock_fd_path" 2>/dev/null) || return 1
    overcrow_fd_mode=$("$stat_program" -L -c '%a' -- \
        "$overcrow_lock_fd_path" 2>/dev/null) || return 1
    overcrow_fd_device=$("$stat_program" -L -c '%d' -- \
        "$overcrow_lock_fd_path" 2>/dev/null) || return 1
    overcrow_fd_inode=$("$stat_program" -L -c '%i' -- \
        "$overcrow_lock_fd_path" 2>/dev/null) || return 1

    [ ! -L "$kwin_lock_path" ] || return 1
    regular_file "$kwin_lock_path" "$current_uid" || return 1
    path_not_mountpoint "$kwin_lock_path" || return 1
    overcrow_path_owner=$("$stat_program" -L -c '%u' -- \
        "$kwin_lock_path" 2>/dev/null) || return 1
    overcrow_path_mode=$("$stat_program" -L -c '%a' -- \
        "$kwin_lock_path" 2>/dev/null) || return 1
    overcrow_path_device=$("$stat_program" -L -c '%d' -- \
        "$kwin_lock_path" 2>/dev/null) || return 1
    overcrow_path_inode=$("$stat_program" -L -c '%i' -- \
        "$kwin_lock_path" 2>/dev/null) || return 1
    overcrow_data_device=$(path_device "$data_home") || return 1

    for overcrow_lock_number in \
        "$overcrow_fd_owner" "$overcrow_fd_mode" \
        "$overcrow_fd_device" "$overcrow_fd_inode" \
        "$overcrow_path_owner" "$overcrow_path_mode" \
        "$overcrow_path_device" "$overcrow_path_inode" \
        "$overcrow_data_device"; do
        case $overcrow_lock_number in
            ''|*[!0-9]*) return 1 ;;
        esac
    done
    [ "$overcrow_fd_owner" = "$current_uid" ] && \
        [ "$overcrow_path_owner" = "$current_uid" ] && \
        [ "$overcrow_fd_mode" = 600 ] && \
        [ "$overcrow_path_mode" = 600 ] && \
        [ "$overcrow_fd_device" = "$overcrow_data_device" ] && \
        [ "$overcrow_path_device" = "$overcrow_data_device" ] && \
        [ "$overcrow_fd_device:$overcrow_fd_inode" = \
            "$overcrow_path_device:$overcrow_path_inode" ]
}

kwin_acquire_lock() {
    if [ ! -e "$kwin_lock_path" ] && [ ! -L "$kwin_lock_path" ]; then
        (
            set -C
            umask 077
            : > "$kwin_lock_path"
        ) 2>/dev/null || return 1
    fi
    regular_file "$kwin_lock_path" "$current_uid" || return 1
    [ "$("$stat_program" -c '%a' -- "$kwin_lock_path" 2>/dev/null)" = 600 ] || return 1
    overcrow_lock_device=$(path_device "$kwin_lock_path") || return 1
    overcrow_data_device=$(path_device "$data_home") || return 1
    [ "$overcrow_lock_device" = "$overcrow_data_device" ] || return 1
    path_not_mountpoint "$kwin_lock_path" || return 1
    exec 9<> "$kwin_lock_path" || return 1
    if ! kwin_open_lock_matches_path; then
        exec 9>&-
        return 1
    fi
    if ! "$flock_program" -n 9; then
        exec 9>&-
        return 1
    fi
    if ! kwin_open_lock_matches_path; then
        exec 9>&-
        return 1
    fi
}

user_config_safe() {
    safe_path "$config_home" || return 1
    direct_directory "$config_home" "$current_uid"
}

kwinrc_safe() {
    overcrow_kwinrc="$config_home/kwinrc"
    [ ! -L "$overcrow_kwinrc" ] || return 1
    [ ! -e "$overcrow_kwinrc" ] || regular_file "$overcrow_kwinrc" "$current_uid"
}

kwin_destination_parents_safe() {
    safe_path "$data_home" || return 1
    direct_directory "$data_home" "$current_uid" || return 1
    overcrow_kwin_dir="$data_home/kwin"
    overcrow_scripts_dir="$overcrow_kwin_dir/scripts"
    for overcrow_directory in "$overcrow_kwin_dir" "$overcrow_scripts_dir"; do
        [ ! -L "$overcrow_directory" ] || return 1
        [ ! -e "$overcrow_directory" ] || \
            direct_directory "$overcrow_directory" "$current_uid" || return 1
    done
}

kwin_destination_absent() {
    kwin_destination_parents_safe || return 1
    [ ! -e "$kwin_destination" ] && [ ! -L "$kwin_destination" ]
}

kwin_destination_valid() {
    kwin_destination_parents_safe || return 1
    overcrow_destination_version=$(kwin_package_version \
        "$kwin_destination" "$current_uid") || return 1
    [ "$overcrow_destination_version" = current ]
}

kwin_destination_version() {
    kwin_destination_parents_safe || return 1
    kwin_package_version "$kwin_destination" "$current_uid"
}

kwin_read_config_value() {
    overcrow_config_value=$(run_bounded "$kreadconfig_program" \
        --file kwinrc \
        --group Plugins \
        --key "${kwin_id}Enabled" \
        --default '__OVERCROW_ABSENT__' 2>/dev/null) || return $?
    case $overcrow_config_value in
        true|false|'__OVERCROW_ABSENT__')
            printf '%s\n' "$overcrow_config_value"
            ;;
        *) return 1 ;;
    esac
}

kwin_write_config_value() {
    case $1 in
        true|false)
            run_bounded "$kwriteconfig_program" \
                --file kwinrc \
                --group Plugins \
                --key "${kwin_id}Enabled" \
                "$1" >/dev/null 2>&1
            ;;
        '__OVERCROW_ABSENT__')
            run_bounded "$kwriteconfig_program" \
                --file kwinrc \
                --group Plugins \
                --key "${kwin_id}Enabled" \
                --delete '' >/dev/null 2>&1
            ;;
        *) return 1 ;;
    esac
}

kwin_remove_exact_destination() {
    overcrow_expected_destination="$data_home/kwin/scripts/$kwin_id"
    [ "$kwin_destination" = "$overcrow_expected_destination" ] || return 1
    kwin_destination_parents_safe || return 1
    [ ! -L "$kwin_destination" ] || return 1
    if [ ! -e "$kwin_destination" ]; then
        return 0
    fi
    kwin_cleanup_tree_safe "$kwin_destination" "$current_uid" || return 1
    "$rm_program" -rf --one-file-system -- "$kwin_destination" || return 1
    [ ! -e "$kwin_destination" ] && [ ! -L "$kwin_destination" ]
}

kwin_remove_reserved_artifact() {
    overcrow_reserved_artifact=$1
    case $overcrow_reserved_artifact in
        "$kwin_restore_build"|"$kwin_restore_ready"|"$kwin_discard") ;;
        *) return 1 ;;
    esac
    kwin_destination_parents_safe || return 1
    if [ ! -e "$overcrow_reserved_artifact" ] && \
        [ ! -L "$overcrow_reserved_artifact" ]; then
        return 0
    fi
    [ ! -L "$overcrow_reserved_artifact" ] || return 1
    kwin_cleanup_tree_safe "$overcrow_reserved_artifact" "$current_uid" || return 1
    "$rm_program" -rf --one-file-system -- "$overcrow_reserved_artifact" || return 1
    [ ! -e "$overcrow_reserved_artifact" ] && \
        [ ! -L "$overcrow_reserved_artifact" ]
}

kwin_transaction_values() {
    overcrow_transaction_root=$1
    direct_directory "$overcrow_transaction_root" "$current_uid" || return 1
    overcrow_transaction_device=$(path_device "$overcrow_transaction_root") || return 1
    overcrow_data_device=$(path_device "$data_home") || return 1
    [ "$overcrow_transaction_device" = "$overcrow_data_device" ] || return 1
    path_not_mountpoint "$overcrow_transaction_root" || return 1
    overcrow_manifest="$overcrow_transaction_root/manifest"
    regular_file "$overcrow_manifest" "$current_uid" || return 1
    [ "$("$stat_program" -c '%a' -- "$overcrow_manifest" 2>/dev/null)" = 600 ] || return 1
    overcrow_manifest_size=$("$stat_program" -c '%s' -- "$overcrow_manifest" 2>/dev/null) || return 1
    case $overcrow_manifest_size in
        ''|*[!0-9]*) return 1 ;;
    esac
    [ "$overcrow_manifest_size" -le 160 ] || return 1
    overcrow_manifest_values=$("$awk_program" '
        NR == 1 && $0 != "OVERCROW_TRANSACTION_V1" { exit 71 }
        NR == 2 {
            if ($0 !~ /^previous_package=(absent|current|legacy-pre-alpha-1|legacy-task7|legacy-mvp)$/) exit 72
            package = substr($0, 18)
        }
        NR == 3 {
            if ($0 !~ /^previous_config=(absent|true|false)$/) exit 73
            config = substr($0, 17)
        }
        NR > 3 { exit 74 }
        END {
            if (NR != 3 || package == "" || config == "") exit 75
            print package "|" config
        }
    ' "$overcrow_manifest") || return 1
    overcrow_transaction_package=${overcrow_manifest_values%%|*}
    if [ "$overcrow_transaction_package" = absent ]; then
        overcrow_unexpected_transaction_entry=$("$find_program" -P \
            "$overcrow_transaction_root" -mindepth 1 \
            ! -path "$overcrow_manifest" -print -quit 2>/dev/null) || return 1
    else
        overcrow_snapshot_version=$(kwin_package_version \
            "$overcrow_transaction_root/package" "$current_uid") || return 1
        [ "$overcrow_snapshot_version" = "$overcrow_transaction_package" ] || return 1
        overcrow_unexpected_transaction_entry=$("$find_program" -P \
            "$overcrow_transaction_root" -mindepth 1 \
            ! \( \
                -path "$overcrow_manifest" -o \
                -path "$overcrow_transaction_root/package" -o \
                -path "$overcrow_transaction_root/package/contents" -o \
                -path "$overcrow_transaction_root/package/contents/code" -o \
                -path "$overcrow_transaction_root/package/contents/code/main.js" -o \
                -path "$overcrow_transaction_root/package/metadata.json" \
            \) -print -quit 2>/dev/null) || return 1
    fi
    [ -z "$overcrow_unexpected_transaction_entry" ] || return 1
    overcrow_mounted_transaction_entry=$("$find_program" -P \
        "$overcrow_transaction_root" \
        -exec "$mountpoint_program" -q -- '{}' \; -print -quit 2>/dev/null) || return 1
    [ -z "$overcrow_mounted_transaction_entry" ] || return 1
    printf '%s\n' "$overcrow_manifest_values"
}

kwin_partial_package_tree_safe() {
    overcrow_partial_root=$1
    overcrow_partial_parent=${overcrow_partial_root%/*}
    direct_directory "$overcrow_partial_parent" "$current_uid" || return 1
    direct_directory "$overcrow_partial_root" "$current_uid" || return 1
    overcrow_partial_device=$(path_device "$overcrow_partial_parent") || return 1
    [ "$(path_device "$overcrow_partial_root")" = "$overcrow_partial_device" ] || return 1
    for overcrow_partial_directory in \
        "$overcrow_partial_root/contents" \
        "$overcrow_partial_root/contents/code"; do
        [ ! -L "$overcrow_partial_directory" ] || return 1
        if [ -e "$overcrow_partial_directory" ]; then
            direct_directory "$overcrow_partial_directory" "$current_uid" || return 1
            [ "$(path_device "$overcrow_partial_directory")" = \
                "$overcrow_partial_device" ] || return 1
        fi
    done
    for overcrow_partial_file in \
        "$overcrow_partial_root/metadata.json" \
        "$overcrow_partial_root/contents/code/main.js"; do
        [ ! -L "$overcrow_partial_file" ] || return 1
        if [ -e "$overcrow_partial_file" ]; then
            regular_file "$overcrow_partial_file" "$current_uid" || return 1
            [ "$(path_device "$overcrow_partial_file")" = \
                "$overcrow_partial_device" ] || return 1
        fi
    done
    overcrow_partial_extra=$("$find_program" -P "$overcrow_partial_root" \
        -mindepth 1 \
        ! \( \
            -path "$overcrow_partial_root/contents" -o \
            -path "$overcrow_partial_root/contents/code" -o \
            -path "$overcrow_partial_root/contents/code/main.js" -o \
            -path "$overcrow_partial_root/metadata.json" \
        \) -print -quit 2>/dev/null) || return 1
    [ -z "$overcrow_partial_extra" ] || return 1
    overcrow_partial_mount=$("$find_program" -P "$overcrow_partial_root" \
        -exec "$mountpoint_program" -q -- '{}' \; -print -quit 2>/dev/null) || return 1
    [ -z "$overcrow_partial_mount" ]
}

kwin_dispose_partial_package_tree() {
    overcrow_partial_root=$1
    kwin_partial_package_tree_safe "$overcrow_partial_root" || return 1
    for overcrow_partial_file in \
        "$overcrow_partial_root/metadata.json" \
        "$overcrow_partial_root/contents/code/main.js"; do
        if [ -e "$overcrow_partial_file" ]; then
            "$rm_program" -f -- "$overcrow_partial_file" || return 1
        fi
    done
    for overcrow_partial_directory in \
        "$overcrow_partial_root/contents/code" \
        "$overcrow_partial_root/contents" \
        "$overcrow_partial_root"; do
        if [ -e "$overcrow_partial_directory" ]; then
            "$rmdir_program" -- "$overcrow_partial_directory" || return 1
        fi
    done
}

kwin_partial_transaction_tree_safe() {
    overcrow_partial_transaction=$1
    overcrow_partial_parent=${overcrow_partial_transaction%/*}
    direct_directory "$overcrow_partial_parent" "$current_uid" || return 1
    direct_directory "$overcrow_partial_transaction" "$current_uid" || return 1
    overcrow_partial_device=$(path_device "$overcrow_partial_parent") || return 1
    [ "$(path_device "$overcrow_partial_transaction")" = \
        "$overcrow_partial_device" ] || return 1
    for overcrow_partial_directory in \
        "$overcrow_partial_transaction/package" \
        "$overcrow_partial_transaction/package/contents" \
        "$overcrow_partial_transaction/package/contents/code"; do
        [ ! -L "$overcrow_partial_directory" ] || return 1
        if [ -e "$overcrow_partial_directory" ]; then
            direct_directory "$overcrow_partial_directory" "$current_uid" || return 1
            [ "$(path_device "$overcrow_partial_directory")" = \
                "$overcrow_partial_device" ] || return 1
        fi
    done
    for overcrow_partial_file in \
        "$overcrow_partial_transaction/manifest" \
        "$overcrow_partial_transaction/package/metadata.json" \
        "$overcrow_partial_transaction/package/contents/code/main.js"; do
        [ ! -L "$overcrow_partial_file" ] || return 1
        if [ -e "$overcrow_partial_file" ]; then
            regular_file "$overcrow_partial_file" "$current_uid" || return 1
            [ "$(path_device "$overcrow_partial_file")" = \
                "$overcrow_partial_device" ] || return 1
        fi
    done
    overcrow_partial_extra=$("$find_program" -P "$overcrow_partial_transaction" \
        -mindepth 1 \
        ! \( \
            -path "$overcrow_partial_transaction/manifest" -o \
            -path "$overcrow_partial_transaction/package" -o \
            -path "$overcrow_partial_transaction/package/contents" -o \
            -path "$overcrow_partial_transaction/package/contents/code" -o \
            -path "$overcrow_partial_transaction/package/contents/code/main.js" -o \
            -path "$overcrow_partial_transaction/package/metadata.json" \
        \) -print -quit 2>/dev/null) || return 1
    [ -z "$overcrow_partial_extra" ] || return 1
    overcrow_partial_mount=$("$find_program" -P "$overcrow_partial_transaction" \
        -exec "$mountpoint_program" -q -- '{}' \; -print -quit 2>/dev/null) || return 1
    [ -z "$overcrow_partial_mount" ]
}

kwin_dispose_transaction_tree() {
    overcrow_disposal_root=$1
    kwin_partial_transaction_tree_safe "$overcrow_disposal_root" || return 1
    if [ -d "$overcrow_disposal_root/package" ]; then
        for overcrow_disposal_file in \
            "$overcrow_disposal_root/package/metadata.json" \
            "$overcrow_disposal_root/package/contents/code/main.js"; do
            if [ -e "$overcrow_disposal_file" ]; then
                "$rm_program" -f -- "$overcrow_disposal_file" || return 1
            fi
        done
        for overcrow_disposal_directory in \
            "$overcrow_disposal_root/package/contents/code" \
            "$overcrow_disposal_root/package/contents" \
            "$overcrow_disposal_root/package"; do
            if [ -e "$overcrow_disposal_directory" ]; then
                "$rmdir_program" -- "$overcrow_disposal_directory" || return 1
            fi
        done
    fi
    if [ -e "$overcrow_disposal_root/manifest" ]; then
        "$rm_program" -f -- "$overcrow_disposal_root/manifest" || return 1
    fi
    "$rmdir_program" -- "$overcrow_disposal_root"
}

kwin_recover_non_authoritative_artifacts() {
    if [ -e "$kwin_transaction_cleanup_dir" ] || \
        [ -L "$kwin_transaction_cleanup_dir" ]; then
        [ ! -e "$kwin_transaction_dir" ] && \
            [ ! -L "$kwin_transaction_dir" ] || return 1
        kwin_dispose_transaction_tree "$kwin_transaction_cleanup_dir" || return 1
    fi
    if [ -e "$kwin_transaction_build" ] || [ -L "$kwin_transaction_build" ]; then
        [ ! -e "$kwin_transaction_dir" ] && \
            [ ! -L "$kwin_transaction_dir" ] || return 1
        kwin_dispose_transaction_tree "$kwin_transaction_build" || return 1
    fi
    if [ -e "$kwin_restore_build" ] || [ -L "$kwin_restore_build" ]; then
        kwin_dispose_partial_package_tree "$kwin_restore_build" || return 1
    fi
}

kwin_rename_no_clobber() {
    overcrow_rename_source=$1
    overcrow_rename_destination=$2
    [ -e "$overcrow_rename_source" ] && [ ! -L "$overcrow_rename_source" ] || return 1
    [ ! -e "$overcrow_rename_destination" ] && \
        [ ! -L "$overcrow_rename_destination" ] || return 1
    "$mv_program" -T --no-clobber -- \
        "$overcrow_rename_source" "$overcrow_rename_destination" || return 1
    [ ! -e "$overcrow_rename_source" ] && \
        [ ! -L "$overcrow_rename_source" ] && \
        [ -e "$overcrow_rename_destination" ] && \
        [ ! -L "$overcrow_rename_destination" ]
}

kwin_transaction_cleanup() {
    if [ ! -e "$kwin_transaction_dir" ] && [ ! -L "$kwin_transaction_dir" ]; then
        return 0
    fi
    kwin_transaction_values "$kwin_transaction_dir" >/dev/null || return 1
    [ ! -e "$kwin_transaction_cleanup_dir" ] && \
        [ ! -L "$kwin_transaction_cleanup_dir" ] || return 1
    kwin_rename_no_clobber \
        "$kwin_transaction_dir" "$kwin_transaction_cleanup_dir" || return 1
    "$sync_program" -f -- "$data_home" || return 1
    kwin_dispose_transaction_tree "$kwin_transaction_cleanup_dir"
}

kwin_transaction_begin() {
    if kwin_destination_absent; then
        kwin_previous_package=absent
    else
        kwin_previous_package=$(kwin_destination_version) || return 1
    fi
    kwin_previous_config=$(kwin_read_config_value) || return $?
    case $kwin_previous_config in
        '__OVERCROW_ABSENT__') overcrow_manifest_config=absent ;;
        true|false) overcrow_manifest_config=$kwin_previous_config ;;
        *) return 1 ;;
    esac
    for overcrow_transaction_path in \
        "$kwin_transaction_dir" \
        "$kwin_transaction_build" \
        "$kwin_transaction_cleanup_dir"; do
        [ ! -e "$overcrow_transaction_path" ] && \
            [ ! -L "$overcrow_transaction_path" ] || return 1
    done
    "$mkdir_program" -m 0700 -- "$kwin_transaction_build" || return 1
    if [ "$kwin_previous_package" != absent ]; then
        "$mkdir_program" -m 0755 -- \
            "$kwin_transaction_build/package" \
            "$kwin_transaction_build/package/contents" \
            "$kwin_transaction_build/package/contents/code" || return 1
        "$cp_program" -p -- "$kwin_destination/metadata.json" \
            "$kwin_transaction_build/package/metadata.json" || return 1
        "$cp_program" -p -- "$kwin_destination/contents/code/main.js" \
            "$kwin_transaction_build/package/contents/code/main.js" || return 1
    fi
    "$install_program" -m 0600 -- /dev/null \
        "$kwin_transaction_build/manifest" || return 1
    printf '%s\n' \
        'OVERCROW_TRANSACTION_V1' \
        "previous_package=$kwin_previous_package" \
        "previous_config=$overcrow_manifest_config" \
        > "$kwin_transaction_build/manifest" || return 1
    kwin_transaction_values "$kwin_transaction_build" >/dev/null || return 1
    "$sync_program" -f -- "$kwin_transaction_build/manifest" || return 1
    if [ "$kwin_previous_package" != absent ]; then
        "$sync_program" -f -- \
            "$kwin_transaction_build/package/metadata.json" \
            "$kwin_transaction_build/package/contents/code/main.js" \
            "$kwin_transaction_build/package/contents/code" \
            "$kwin_transaction_build/package/contents" \
            "$kwin_transaction_build/package" || return 1
    fi
    "$sync_program" -f -- "$kwin_transaction_build" || return 1
    kwin_rename_no_clobber \
        "$kwin_transaction_build" "$kwin_transaction_dir" || return 1
    "$sync_program" -f -- "$data_home" || return 1
    kwin_transaction_values "$kwin_transaction_dir" >/dev/null
}

kwin_build_restore_staging() {
    overcrow_expected_restore_version=$1
    [ ! -e "$kwin_restore_build" ] && \
        [ ! -L "$kwin_restore_build" ] || return 1
    [ ! -e "$kwin_restore_ready" ] && \
        [ ! -L "$kwin_restore_ready" ] || return 1
    "$mkdir_program" -m 0755 -- \
        "$kwin_restore_build" \
        "$kwin_restore_build/contents" \
        "$kwin_restore_build/contents/code" || return 1
    if ! "$install_program" -m 0644 -- \
        "$kwin_transaction_dir/package/metadata.json" \
        "$kwin_restore_build/metadata.json" || \
        ! "$install_program" -m 0644 -- \
            "$kwin_transaction_dir/package/contents/code/main.js" \
            "$kwin_restore_build/contents/code/main.js"; then
        kwin_dispose_partial_package_tree "$kwin_restore_build" || true
        return 1
    fi
    overcrow_staging_version=$(kwin_package_version \
        "$kwin_restore_build" "$current_uid") || return 1
    [ "$overcrow_staging_version" = "$overcrow_expected_restore_version" ] || return 1
    kwin_rename_no_clobber "$kwin_restore_build" "$kwin_restore_ready" || return 1
    overcrow_staging_version=$(kwin_package_version \
        "$kwin_restore_ready" "$current_uid") || return 1
    [ "$overcrow_staging_version" = "$overcrow_expected_restore_version" ]
}

kwin_recovery_artifacts_absent() {
    for overcrow_recovery_path in \
        "$kwin_transaction_dir" \
        "$kwin_transaction_build" \
        "$kwin_transaction_cleanup_dir" \
        "$kwin_restore_build" \
        "$kwin_restore_ready" \
        "$kwin_discard"; do
        [ ! -e "$overcrow_recovery_path" ] && \
            [ ! -L "$overcrow_recovery_path" ] || return 1
    done
}

kwin_restore_recorded_package() {
    overcrow_recovery_version=$1
    if [ "$overcrow_recovery_version" = absent ]; then
        for overcrow_absent_artifact in \
            "$kwin_restore_build" "$kwin_restore_ready" "$kwin_discard"; do
            [ ! -e "$overcrow_absent_artifact" ] && \
                [ ! -L "$overcrow_absent_artifact" ] || return 1
        done
        if kwin_destination_absent; then
            return 0
        fi
        run_bounded "$kpackagetool_program" \
            --type KWin/Script --remove "$kwin_id" >/dev/null 2>&1 || return $?
        kwin_remove_exact_destination
        return
    fi
    overcrow_destination_version=$(kwin_destination_version 2>/dev/null) || \
        overcrow_destination_version=unknown
    if [ "$overcrow_destination_version" = "$overcrow_recovery_version" ]; then
        if [ -e "$kwin_restore_ready" ] || [ -L "$kwin_restore_ready" ]; then
            kwin_remove_reserved_artifact "$kwin_restore_ready" || return 1
        fi
        if [ -e "$kwin_discard" ] || [ -L "$kwin_discard" ]; then
            kwin_remove_reserved_artifact "$kwin_discard" || return 1
        fi
        return 0
    fi
    if [ ! -e "$kwin_restore_ready" ] && [ ! -L "$kwin_restore_ready" ]; then
        kwin_build_restore_staging "$overcrow_recovery_version" || return 1
    else
        overcrow_ready_version=$(kwin_package_version \
            "$kwin_restore_ready" "$current_uid") || return 1
        [ "$overcrow_ready_version" = "$overcrow_recovery_version" ] || return 1
    fi
    if [ -e "$kwin_destination" ] || [ -L "$kwin_destination" ]; then
        [ ! -e "$kwin_discard" ] && [ ! -L "$kwin_discard" ] || return 1
        [ ! -L "$kwin_destination" ] || return 1
        kwin_cleanup_tree_safe "$kwin_destination" "$current_uid" || return 1
        kwin_rename_no_clobber "$kwin_destination" "$kwin_discard" || return 1
    fi
    [ ! -e "$kwin_destination" ] && [ ! -L "$kwin_destination" ] || return 1
    kwin_rename_no_clobber "$kwin_restore_ready" "$kwin_destination" || return 1
    overcrow_recovered_version=$(kwin_destination_version) || return 1
    [ "$overcrow_recovered_version" = "$overcrow_recovery_version" ] || return 1
    if [ -e "$kwin_discard" ] || [ -L "$kwin_discard" ]; then
        kwin_remove_reserved_artifact "$kwin_discard" || return 1
    fi
}

kwin_recover_pending_transaction() {
    kwin_recover_non_authoritative_artifacts || return 1
    if [ ! -e "$kwin_transaction_dir" ] && [ ! -L "$kwin_transaction_dir" ]; then
        kwin_recovery_artifacts_absent
        return
    fi
    overcrow_manifest_values=$(kwin_transaction_values "$kwin_transaction_dir") || return 1
    kwin_previous_package=${overcrow_manifest_values%%|*}
    overcrow_manifest_config=${overcrow_manifest_values#*|}
    kwin_restore_recorded_package "$kwin_previous_package" || return $?
    case $overcrow_manifest_config in
        absent) kwin_previous_config='__OVERCROW_ABSENT__' ;;
        true|false) kwin_previous_config=$overcrow_manifest_config ;;
        *) return 1 ;;
    esac
    kwin_write_config_value "$kwin_previous_config" || return $?
    overcrow_restored_config=$(kwin_read_config_value) || return $?
    [ "$overcrow_restored_config" = "$kwin_previous_config" ] || return 1
    kwin_transaction_cleanup
}

append_rollback_error() {
    if [ -n "$kwin_rollback_error" ]; then
        kwin_rollback_error="$kwin_rollback_error; $1"
    else
        kwin_rollback_error=$1
    fi
}

kwin_transaction_rollback() {
    kwin_rollback_error=
    if kwin_recover_pending_transaction; then
        :
    else
        overcrow_rollback_status=$?
        if bounded_status_is_fatal "$overcrow_rollback_status"; then
            return "$overcrow_rollback_status"
        fi
        append_rollback_error 'published package/KConfig recovery failed'
    fi
    [ -z "$kwin_rollback_error" ]
}

kwin_package_is_exact() {
    overcrow_source_version=$(kwin_package_version \
        "$kwin_package" "$resource_owner") || return 1
    [ "$overcrow_source_version" = current ]
}

hyprland_ready() {
    user_config_safe && hyprland_config_ready "$config_home" "$hyprland_templates"
}

kwin_ready() {
    user_config_safe || return 1
    kwinrc_safe || return 1
    kwin_package_is_exact || return 1
    kwin_recovery_artifacts_absent || return 1
    kwin_destination_valid || return 1
    trusted_plasma_programs_ready || return 1
    run_bounded "$kpackagetool_program" \
        --type KWin/Script --show "$kwin_id" >/dev/null 2>&1 || return $?
    overcrow_enabled=$(run_bounded "$kreadconfig_program" \
        --file kwinrc \
        --group Plugins \
        --key "${kwin_id}Enabled" 2>/dev/null) || return $?
    [ "$overcrow_enabled" = true ]
}

if [ "$action" = status ]; then
    overcrow_status_exit=0
    if [ -n "$desktop_error" ]; then
        status_result false "$desktop_error"
    elif [ "$desktop" = hyprland ]; then
        if hyprland_ready; then
            status_result true
        else
            status_result false 'Hyprland integration is missing or unsafe'
        fi
    else
        if kwin_ready; then
            status_result true
        else
            overcrow_status_code=$?
            if bounded_status_is_fatal "$overcrow_status_code"; then
                status_result false 'bounded command could not be started during Plasma status'
                overcrow_status_exit=125
            else
                status_result false 'Plasma integration or status tool is unavailable'
            fi
        fi
    fi
    exit "$overcrow_status_exit"
fi

[ -z "$desktop_error" ] || fail "$desktop_error"
user_config_safe || fail 'refusing a symlinked or foreign-owned user configuration directory'

if [ "$desktop" = hyprland ]; then
    if install_hyprland_config "$config_home" "$hyprland_templates"; then
        exit 0
    else
        overcrow_hyprland_status=$?
    fi
    if bounded_status_is_fatal "$overcrow_hyprland_status"; then
        exit 125
    fi
    fail 'failed to install the managed Hyprland fragment'
fi

kwin_package_is_exact || fail 'KWin package metadata does not contain the exact OverCrow ID'
kwinrc_safe || fail 'refusing an unsafe kwinrc destination'
kwin_destination_parents_safe || fail 'refusing an unsafe KWin package destination'
trusted_plasma_programs_ready || fail 'required Plasma integration tools are unavailable or unsafe'
kwin_acquire_lock || fail 'another Plasma integration transaction is active or the lock is unsafe'
if kwin_recover_pending_transaction; then
    :
else
    overcrow_recovery_status=$?
    if bounded_status_is_fatal "$overcrow_recovery_status"; then
        fail_bounded_status 'Plasma transaction recovery'
    fi
    fail 'refusing an unknown or unrecoverable KWin transaction artifact state'
fi
if ! kwin_destination_absent; then
    kwin_destination_version >/dev/null || \
        fail 'refusing an unsafe package/config state'
    if run_bounded "$kpackagetool_program" \
        --type KWin/Script --show "$kwin_id" >/dev/null 2>&1; then
        :
    else
        overcrow_query_status=$?
        if bounded_status_is_fatal "$overcrow_query_status"; then
            fail_bounded_status 'existing Plasma package query'
        fi
        fail 'failed to query the existing exact OverCrow KWin package'
    fi
fi
if kwin_transaction_begin; then
    :
else
    overcrow_begin_status=$?
    if bounded_status_is_fatal "$overcrow_begin_status"; then
        fail_bounded_status 'Plasma transaction snapshot'
    fi
    kwin_transaction_cleanup || true
    fail 'refusing an unsafe package/config state or failing to create its exact snapshot'
fi

kwin_transaction_error=
if [ "$kwin_previous_package" = absent ]; then
    if run_bounded "$kpackagetool_program" \
        --type KWin/Script --install "$kwin_package" >/dev/null 2>&1; then
        :
    else
        overcrow_install_status=$?
        if bounded_status_is_fatal "$overcrow_install_status"; then
            fail_bounded_status 'Plasma package install'
        fi
        kwin_transaction_error='failed to install the exact OverCrow KWin package'
    fi
elif [ "$kwin_previous_package" = legacy-pre-alpha-1 ] || \
    [ "$kwin_previous_package" = legacy-task7 ] || \
    [ "$kwin_previous_package" = legacy-mvp ]; then
    if run_bounded "$kpackagetool_program" \
        --type KWin/Script --upgrade "$kwin_package" >/dev/null 2>&1; then
        :
    else
        overcrow_upgrade_status=$?
        if bounded_status_is_fatal "$overcrow_upgrade_status"; then
            fail_bounded_status 'Plasma package upgrade'
        fi
        kwin_transaction_error='failed to upgrade the exact OverCrow KWin package'
    fi
else
    :
fi

if [ -z "$kwin_transaction_error" ]; then
    overcrow_post_mutation_version=$(kwin_destination_version) || \
        overcrow_post_mutation_version=
    if [ "$overcrow_post_mutation_version" != current ]; then
        kwin_transaction_error='exact OverCrow KWin package verification failed after mutation'
    fi
fi
if [ -z "$kwin_transaction_error" ]; then
    if kwin_write_config_value true; then
        if overcrow_enabled=$(kwin_read_config_value); then
            if [ "$overcrow_enabled" != true ]; then
                kwin_transaction_error='exact OverCrow KWin plugin enablement verification failed'
            fi
        else
            overcrow_read_status=$?
            if bounded_status_is_fatal "$overcrow_read_status"; then
                fail_bounded_status 'Plasma enablement verification'
            fi
            kwin_transaction_error='exact OverCrow KWin plugin enablement verification failed'
        fi
    else
        overcrow_write_status=$?
        if bounded_status_is_fatal "$overcrow_write_status"; then
            fail_bounded_status 'Plasma KConfig enablement'
        fi
        kwin_transaction_error='failed to enable the exact OverCrow KWin plugin'
    fi
fi

if [ -n "$kwin_transaction_error" ]; then
    if kwin_transaction_rollback; then
        fail "$kwin_transaction_error; transaction rolled back"
    else
        overcrow_rollback_status=$?
        if bounded_status_is_fatal "$overcrow_rollback_status"; then
            fail_bounded_status 'Plasma transaction rollback'
        fi
        fail "$kwin_transaction_error; rollback failed: $kwin_rollback_error"
    fi
fi

if [ "$qdbus_ready" = true ]; then
    if run_bounded "$qdbus_program" \
        org.kde.KWin /KWin reconfigure >/dev/null 2>&1; then
        :
    else
        overcrow_qdbus_status=$?
        if bounded_status_is_fatal "$overcrow_qdbus_status"; then
            fail_bounded_status 'Plasma compositor notification'
        fi
        printf '%s\n' 'warning: durable KWin integration succeeded but reconfigure failed' >&2
    fi
fi
kwin_transaction_cleanup || fail 'integration succeeded but transaction snapshot cleanup failed'
