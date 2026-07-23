#!/bin/sh

overcrow_hypr_system_program_parent='/usr/bin'
overcrow_hypr_system_program_owner=0
overcrow_hypr_compositor_program_parent='/usr/bin'
overcrow_hypr_compositor_program_owner=0
overcrow_hyprctl_program='/usr/bin/hyprctl'
overcrow_hypr_id_program='/usr/bin/id'
overcrow_hypr_timeout_program='/usr/bin/timeout'
overcrow_hypr_readlink_program='/usr/bin/readlink'
overcrow_hypr_stat_program='/usr/bin/stat'
overcrow_hypr_cp_program='/usr/bin/cp'
overcrow_hypr_rm_program='/usr/bin/rm'
overcrow_hypr_mktemp_program='/usr/bin/mktemp'
overcrow_hypr_install_program='/usr/bin/install'
overcrow_hypr_mv_program='/usr/bin/mv'
overcrow_hypr_cmp_program='/usr/bin/cmp'
overcrow_hypr_dirname_program='/usr/bin/dirname'
overcrow_hypr_chmod_program='/usr/bin/chmod'
overcrow_hypr_date_program='/usr/bin/date'
overcrow_hypr_awk_program='/usr/bin/gawk'
overcrow_hypr_sed_program='/usr/bin/sed'
overcrow_hypr_grep_program='/usr/bin/grep'

overcrow_hypr_trusted_executable() {
    overcrow_program=$1
    [ "${overcrow_program%/*}" = "$overcrow_hypr_system_program_parent" ] || return 1
    [ -f "$overcrow_program" ] && [ ! -L "$overcrow_program" ] && \
        [ -x "$overcrow_program" ] || return 1
    overcrow_program_real=$("$overcrow_hypr_readlink_program" -f -- \
        "$overcrow_program" 2>/dev/null) || return 1
    [ "$overcrow_program_real" = "$overcrow_program" ] || return 1
    overcrow_program_owner=$("$overcrow_hypr_stat_program" -c '%u' -- \
        "$overcrow_program" 2>/dev/null) || return 1
    [ "$overcrow_program_owner" = "$overcrow_hypr_system_program_owner" ] || return 1
    overcrow_program_permissions=$("$overcrow_hypr_stat_program" -c '%A' -- \
        "$overcrow_program" 2>/dev/null) || return 1
    case $overcrow_program_permissions in
        ?????w????|????????w?) return 1 ;;
    esac
}

overcrow_hypr_system_programs_ready() {
    [ "${overcrow_hypr_readlink_program%/*}" = \
        "$overcrow_hypr_system_program_parent" ] || return 1
    [ "${overcrow_hypr_stat_program%/*}" = \
        "$overcrow_hypr_system_program_parent" ] || return 1
    [ -d "$overcrow_hypr_system_program_parent" ] && \
        [ ! -L "$overcrow_hypr_system_program_parent" ] || return 1
    overcrow_program_parent_real=$("$overcrow_hypr_readlink_program" -f -- \
        "$overcrow_hypr_system_program_parent" 2>/dev/null) || return 1
    [ "$overcrow_program_parent_real" = \
        "$overcrow_hypr_system_program_parent" ] || return 1
    overcrow_program_parent_owner=$("$overcrow_hypr_stat_program" -c '%u' -- \
        "$overcrow_hypr_system_program_parent" 2>/dev/null) || return 1
    [ "$overcrow_program_parent_owner" = \
        "$overcrow_hypr_system_program_owner" ] || return 1
    overcrow_program_parent_permissions=$("$overcrow_hypr_stat_program" -c '%A' -- \
        "$overcrow_hypr_system_program_parent" 2>/dev/null) || return 1
    case $overcrow_program_parent_permissions in
        ?????w????|????????w?) return 1 ;;
    esac
    for overcrow_program in \
        "$overcrow_hypr_id_program" \
        "$overcrow_hypr_timeout_program" \
        "$overcrow_hypr_readlink_program" \
        "$overcrow_hypr_stat_program" \
        "$overcrow_hypr_cp_program" \
        "$overcrow_hypr_rm_program" \
        "$overcrow_hypr_mktemp_program" \
        "$overcrow_hypr_install_program" \
        "$overcrow_hypr_mv_program" \
        "$overcrow_hypr_cmp_program" \
        "$overcrow_hypr_dirname_program" \
        "$overcrow_hypr_chmod_program" \
        "$overcrow_hypr_date_program" \
        "$overcrow_hypr_awk_program" \
        "$overcrow_hypr_sed_program" \
        "$overcrow_hypr_grep_program"; do
        overcrow_hypr_trusted_executable "$overcrow_program" || return 1
    done
}

# Returns 0 for a trusted fixed Hyprland command, 1 when it is absent, and 2
# when the fixed path or its authority parent exists but is unsafe.
overcrow_hyprctl_ready() {
    overcrow_hypr_system_programs_ready || return 2
    if [ ! -e "$overcrow_hyprctl_program" ] && \
        [ ! -L "$overcrow_hyprctl_program" ]; then
        return 1
    fi
    [ "${overcrow_hyprctl_program%/*}" = \
        "$overcrow_hypr_compositor_program_parent" ] || return 2
    [ -d "$overcrow_hypr_compositor_program_parent" ] && \
        [ ! -L "$overcrow_hypr_compositor_program_parent" ] || return 2
    overcrow_compositor_parent_real=$("$overcrow_hypr_readlink_program" -f -- \
        "$overcrow_hypr_compositor_program_parent" 2>/dev/null) || return 2
    [ "$overcrow_compositor_parent_real" = \
        "$overcrow_hypr_compositor_program_parent" ] || return 2
    overcrow_compositor_parent_owner=$("$overcrow_hypr_stat_program" -c '%u' -- \
        "$overcrow_hypr_compositor_program_parent" 2>/dev/null) || return 2
    [ "$overcrow_compositor_parent_owner" = \
        "$overcrow_hypr_compositor_program_owner" ] || return 2
    overcrow_compositor_parent_permissions=$("$overcrow_hypr_stat_program" -c '%A' -- \
        "$overcrow_hypr_compositor_program_parent" 2>/dev/null) || return 2
    case $overcrow_compositor_parent_permissions in
        ?????w????|????????w?) return 2 ;;
    esac
    [ -f "$overcrow_hyprctl_program" ] && \
        [ ! -L "$overcrow_hyprctl_program" ] && \
        [ -x "$overcrow_hyprctl_program" ] || return 2
    overcrow_hyprctl_real=$("$overcrow_hypr_readlink_program" -f -- \
        "$overcrow_hyprctl_program" 2>/dev/null) || return 2
    [ "$overcrow_hyprctl_real" = "$overcrow_hyprctl_program" ] || return 2
    overcrow_hyprctl_owner=$("$overcrow_hypr_stat_program" -c '%u' -- \
        "$overcrow_hyprctl_program" 2>/dev/null) || return 2
    [ "$overcrow_hyprctl_owner" = \
        "$overcrow_hypr_compositor_program_owner" ] || return 2
    overcrow_hyprctl_permissions=$("$overcrow_hypr_stat_program" -c '%A' -- \
        "$overcrow_hyprctl_program" 2>/dev/null) || return 2
    case $overcrow_hyprctl_permissions in
        ?????w????|????????w?) return 2 ;;
    esac
}

overcrow_hypr_safe_path() {
    case $1 in
        /*) ;;
        *) return 1 ;;
    esac
    case $1 in
        *[!A-Za-z0-9_./-]*) return 1 ;;
    esac
    return 0
}

overcrow_hypr_user_owned() {
    overcrow_hypr_system_programs_ready || return 1
    [ -L "$1" ] && return 1
    [ ! -e "$1" ] && return 0
    overcrow_owner=$("$overcrow_hypr_stat_program" -c '%u' -- "$1" 2>/dev/null) || return 1
    overcrow_current_uid=$("$overcrow_hypr_id_program" -u 2>/dev/null) || return 1
    case $overcrow_owner:$overcrow_current_uid in
        *[!0-9:]*|:|*:)
            return 1
            ;;
    esac
    [ "$overcrow_owner" -eq "$overcrow_current_uid" ] || return 1
    overcrow_permissions=$("$overcrow_hypr_stat_program" -c '%A' -- "$1" 2>/dev/null) || return 1
    case $overcrow_permissions in
        ?????w????|????????w?) return 1 ;;
    esac
    return 0
}

overcrow_hypr_run_bounded() {
    "$overcrow_hypr_timeout_program" --signal=TERM --kill-after=1s 2s "$@"
}

overcrow_hypr_status_is_fatal() {
    [ "$1" -eq 125 ]
}

overcrow_hypr_direct_directory() {
    [ -d "$1" ] || return 1
    [ ! -L "$1" ] || return 1
    overcrow_directory_real=$("$overcrow_hypr_readlink_program" -f -- "$1" 2>/dev/null) || return 1
    [ "$overcrow_directory_real" = "$1" ] || return 1
    overcrow_hypr_user_owned "$1"
}

overcrow_hypr_backup() {
    overcrow_backup_source=$1
    [ -e "$overcrow_backup_source" ] || return 0
    overcrow_backup_stamp=$("$overcrow_hypr_date_program" '+%Y%m%dT%H%M%S') || return 1
    overcrow_backup_path="${overcrow_backup_source}.overcrow-backup-${overcrow_backup_stamp}"
    overcrow_backup_suffix=0
    while [ -e "$overcrow_backup_path" ]; do
        overcrow_backup_suffix=$((overcrow_backup_suffix + 1))
        overcrow_backup_path="${overcrow_backup_source}.overcrow-backup-${overcrow_backup_stamp}-${overcrow_backup_suffix}"
    done
    "$overcrow_hypr_cp_program" -p -- "$overcrow_backup_source" "$overcrow_backup_path"
}

overcrow_hypr_strip_block() {
    overcrow_strip_source=$1
    overcrow_strip_target=$2
    overcrow_strip_begin=$3
    overcrow_strip_end=$4
    # shellcheck disable=SC2016 # This is an awk program, not shell expansion.
    "$overcrow_hypr_awk_program" -v begin="$overcrow_strip_begin" -v end="$overcrow_strip_end" '
        $0 == begin {
            if (inside) exit 71
            inside = 1
            next
        }
        $0 == end {
            if (!inside) exit 72
            inside = 0
            next
        }
        !inside { print }
        END { if (inside) exit 73 }
    ' "$overcrow_strip_source" > "$overcrow_strip_target"
}

overcrow_hypr_has_exact_block() {
    overcrow_block_source=$1
    overcrow_block_begin=$2
    overcrow_block_link=$3
    overcrow_block_end=$4
    # shellcheck disable=SC2016 # This is an awk program, not shell expansion.
    "$overcrow_hypr_awk_program" -v begin="$overcrow_block_begin" -v link="$overcrow_block_link" -v end="$overcrow_block_end" '
        $0 == begin {
            if (seen || inside) exit 71
            seen = 1
            inside = 1
            next
        }
        $0 == end {
            if (!inside || !linked) exit 72
            inside = 0
            ended = 1
            next
        }
        inside {
            if ($0 != link || linked) exit 73
            linked = 1
        }
        END {
            if (!seen || !linked || !ended || inside) exit 74
        }
    ' "$overcrow_block_source" >/dev/null
}

overcrow_hypr_restore_install() {
    overcrow_restore_main=$1
    overcrow_restore_main_copy=$2
    overcrow_restore_fragment=$3
    overcrow_restore_fragment_copy=$4
    overcrow_restore_fragment_existed=$5

    overcrow_hypr_restore_error=
    if ! "$overcrow_hypr_cp_program" -p -- \
        "$overcrow_restore_main_copy" "$overcrow_restore_main"; then
        overcrow_hypr_restore_error='main restore copy failed'
    elif ! "$overcrow_hypr_cmp_program" -s -- \
        "$overcrow_restore_main_copy" "$overcrow_restore_main"; then
        overcrow_hypr_restore_error='main restore verification failed'
    fi
    if [ "$overcrow_restore_fragment_existed" = true ]; then
        if ! "$overcrow_hypr_cp_program" -p -- \
            "$overcrow_restore_fragment_copy" "$overcrow_restore_fragment"; then
            if [ -n "$overcrow_hypr_restore_error" ]; then
                overcrow_hypr_restore_error="$overcrow_hypr_restore_error; fragment restore copy failed"
            else
                overcrow_hypr_restore_error='fragment restore copy failed'
            fi
        elif ! "$overcrow_hypr_cmp_program" -s -- \
            "$overcrow_restore_fragment_copy" "$overcrow_restore_fragment"; then
            if [ -n "$overcrow_hypr_restore_error" ]; then
                overcrow_hypr_restore_error="$overcrow_hypr_restore_error; fragment restore verification failed"
            else
                overcrow_hypr_restore_error='fragment restore verification failed'
            fi
        fi
    else
        if ! "$overcrow_hypr_rm_program" -f -- "$overcrow_restore_fragment"; then
            if [ -n "$overcrow_hypr_restore_error" ]; then
                overcrow_hypr_restore_error="$overcrow_hypr_restore_error; fragment restore removal failed"
            else
                overcrow_hypr_restore_error='fragment restore removal failed'
            fi
        elif [ -e "$overcrow_restore_fragment" ] || \
            [ -L "$overcrow_restore_fragment" ]; then
            if [ -n "$overcrow_hypr_restore_error" ]; then
                overcrow_hypr_restore_error="$overcrow_hypr_restore_error; fragment absence verification failed"
            else
                overcrow_hypr_restore_error='fragment absence verification failed'
            fi
        fi
    fi
    [ -z "$overcrow_hypr_restore_error" ]
}

overcrow_hypr_cleanup_install_temporaries() {
    overcrow_hypr_cleanup_error=
    for overcrow_hypr_cleanup_path in "$@"; do
        [ -n "$overcrow_hypr_cleanup_path" ] || continue
        if [ -e "$overcrow_hypr_cleanup_path" ] || \
            [ -L "$overcrow_hypr_cleanup_path" ]; then
            if ! "$overcrow_hypr_rm_program" -f -- \
                "$overcrow_hypr_cleanup_path"; then
                if [ -n "$overcrow_hypr_cleanup_error" ]; then
                    overcrow_hypr_cleanup_error="$overcrow_hypr_cleanup_error; failed to remove $overcrow_hypr_cleanup_path"
                else
                    overcrow_hypr_cleanup_error="failed to remove $overcrow_hypr_cleanup_path"
                fi
            elif [ -e "$overcrow_hypr_cleanup_path" ] || \
                [ -L "$overcrow_hypr_cleanup_path" ]; then
                if [ -n "$overcrow_hypr_cleanup_error" ]; then
                    overcrow_hypr_cleanup_error="$overcrow_hypr_cleanup_error; failed to verify removal of $overcrow_hypr_cleanup_path"
                else
                    overcrow_hypr_cleanup_error="failed to verify removal of $overcrow_hypr_cleanup_path"
                fi
            fi
        fi
    done
    [ -z "$overcrow_hypr_cleanup_error" ]
}

install_hyprland_config() {
    overcrow_config_home=$1
    overcrow_template_dir=$2

    if ! overcrow_hypr_safe_path "$overcrow_config_home" || \
        ! overcrow_hypr_safe_path "$overcrow_template_dir"; then
        printf '%s\n' 'error: Hyprland integration paths contain unsupported characters' >&2
        return 1
    fi
    if ! overcrow_hypr_system_programs_ready; then
        printf '%s\n' 'error: trusted system programs are unavailable' >&2
        return 1
    fi

    overcrow_hypr_dir="$overcrow_config_home/hypr"
    if ! overcrow_hypr_direct_directory "$overcrow_config_home" || \
        ! overcrow_hypr_direct_directory "$overcrow_hypr_dir"; then
        printf '%s\n' 'error: refusing a symlinked or foreign-owned Hyprland configuration directory' >&2
        return 1
    fi
    overcrow_lua_main="$overcrow_hypr_dir/hyprland.lua"
    overcrow_conf_main="$overcrow_hypr_dir/hyprland.conf"
    if [ -f "$overcrow_lua_main" ]; then
        overcrow_main=$overcrow_lua_main
        overcrow_fragment="$overcrow_hypr_dir/overcrow.lua"
        overcrow_template="$overcrow_template_dir/overcrow.lua.in"
        overcrow_begin='-- BEGIN OVERCROW MANAGED'
        overcrow_end='-- END OVERCROW MANAGED'
        overcrow_link='require("overcrow")'
    elif [ -f "$overcrow_conf_main" ]; then
        overcrow_main=$overcrow_conf_main
        overcrow_fragment="$overcrow_hypr_dir/overcrow.conf"
        overcrow_template="$overcrow_template_dir/overcrow.conf.in"
        overcrow_begin='# BEGIN OVERCROW MANAGED'
        overcrow_end='# END OVERCROW MANAGED'
        overcrow_link="source = $overcrow_fragment"
    else
        printf '%s\n' "error: no supported Hyprland config found below $overcrow_hypr_dir" >&2
        return 1
    fi

    if [ ! -f "$overcrow_template" ] || [ -L "$overcrow_template" ] || \
        [ ! -r "$overcrow_template" ]; then
        printf '%s\n' "error: missing Hyprland template $overcrow_template" >&2
        return 1
    fi
    if ! overcrow_hypr_user_owned "$overcrow_main" || \
        ! overcrow_hypr_user_owned "$overcrow_fragment"; then
        printf '%s\n' 'error: refusing to replace a Hyprland configuration file not owned directly by this user' >&2
        return 1
    fi
    if [ -e "$overcrow_fragment" ]; then
        case $overcrow_fragment in
            *.conf) overcrow_managed_signature='# Managed by OverCrow. Changes are replaced by the installer.' ;;
            *.lua) overcrow_managed_signature='-- Managed by OverCrow. Changes are replaced by the installer.' ;;
        esac
        overcrow_fragment_header=$("$overcrow_hypr_sed_program" -n \
            '1p' "$overcrow_fragment") || return 1
        if [ "$overcrow_fragment_header" != "$overcrow_managed_signature" ]; then
            printf '%s\n' "error: refusing to overwrite unmanaged $overcrow_fragment" >&2
            return 1
        fi
    fi

    overcrow_live=false
    overcrow_errors_before=
    if overcrow_hyprctl_ready; then
        if overcrow_errors_before=$(overcrow_hypr_run_bounded \
            "$overcrow_hyprctl_program" configerrors 2>/dev/null); then
            overcrow_live=true
        else
            overcrow_query_status=$?
            if overcrow_hypr_status_is_fatal "$overcrow_query_status"; then
                printf '%s\n' 'error: bounded Hyprland preflight could not be started; no files changed' >&2
                return 125
            fi
            printf '%s\n' 'error: failed to query Hyprland configuration safely' >&2
            return 1
        fi
    else
        overcrow_hyprctl_status=$?
        if [ "$overcrow_hyprctl_status" -eq 2 ]; then
            printf '%s\n' 'error: fixed Hyprland command is unsafe' >&2
            return 1
        fi
    fi

    overcrow_rendered=$("$overcrow_hypr_mktemp_program" \
        "$overcrow_hypr_dir/.overcrow-rendered.XXXXXX") || {
        printf '%s\n' 'error: failed to create Hyprland rendered temporary file' >&2
        return 1
    }
    overcrow_main_new=$("$overcrow_hypr_mktemp_program" \
        "$overcrow_hypr_dir/.overcrow-main-new.XXXXXX") || {
        overcrow_hypr_cleanup_install_temporaries "$overcrow_rendered" || true
        printf '%s\n' 'error: failed to create Hyprland main-new temporary file' >&2
        return 1
    }
    if ! "$overcrow_hypr_cp_program" -- "$overcrow_template" "$overcrow_rendered" || \
        ! overcrow_hypr_strip_block \
            "$overcrow_main" "$overcrow_main_new" "$overcrow_begin" "$overcrow_end"; then
        overcrow_hypr_cleanup_install_temporaries \
            "$overcrow_rendered" "$overcrow_main_new" || true
        printf '%s\n' 'error: failed to render Hyprland configuration temporaries' >&2
        return 1
    fi
    if ! {
        printf '\n%s\n' "$overcrow_begin"
        printf '%s\n' "$overcrow_link"
        printf '%s\n' "$overcrow_end"
    } >> "$overcrow_main_new"; then
        overcrow_hypr_cleanup_install_temporaries \
            "$overcrow_rendered" "$overcrow_main_new" || true
        printf '%s\n' 'error: failed to finalize Hyprland main temporary file' >&2
        return 1
    fi
    if ! "$overcrow_hypr_chmod_program" --reference="$overcrow_main" \
        "$overcrow_main_new"; then
        overcrow_hypr_cleanup_install_temporaries \
            "$overcrow_rendered" "$overcrow_main_new" || true
        printf '%s\n' 'error: failed to prepare Hyprland main temporary permissions' >&2
        return 1
    fi

    if ! overcrow_hypr_backup "$overcrow_main" || \
        ! overcrow_hypr_backup "$overcrow_fragment"; then
        overcrow_hypr_cleanup_install_temporaries \
            "$overcrow_rendered" "$overcrow_main_new" || true
        printf '%s\n' 'error: failed to back up Hyprland configuration' >&2
        return 1
    fi

    overcrow_main_copy=$("$overcrow_hypr_mktemp_program" \
        "$overcrow_hypr_dir/.overcrow-main-rollback.XXXXXX") || {
        overcrow_hypr_cleanup_install_temporaries \
            "$overcrow_rendered" "$overcrow_main_new" || true
        printf '%s\n' 'error: failed to create Hyprland main rollback copy' >&2
        return 1
    }
    overcrow_fragment_copy=$("$overcrow_hypr_mktemp_program" \
        "$overcrow_hypr_dir/.overcrow-fragment-rollback.XXXXXX") || {
        overcrow_hypr_cleanup_install_temporaries \
            "$overcrow_rendered" "$overcrow_main_new" "$overcrow_main_copy" || true
        printf '%s\n' 'error: failed to create Hyprland fragment rollback copy' >&2
        return 1
    }
    if ! "$overcrow_hypr_cp_program" -p -- \
        "$overcrow_main" "$overcrow_main_copy"; then
        overcrow_hypr_cleanup_install_temporaries \
            "$overcrow_rendered" "$overcrow_main_new" \
            "$overcrow_main_copy" "$overcrow_fragment_copy" || true
        printf '%s\n' 'error: failed to snapshot Hyprland main configuration' >&2
        return 1
    fi
    overcrow_fragment_existed=false
    if [ -e "$overcrow_fragment" ]; then
        overcrow_fragment_existed=true
        if ! "$overcrow_hypr_cp_program" -p -- \
            "$overcrow_fragment" "$overcrow_fragment_copy"; then
            overcrow_hypr_cleanup_install_temporaries \
                "$overcrow_rendered" "$overcrow_main_new" \
                "$overcrow_main_copy" "$overcrow_fragment_copy" || true
            printf '%s\n' 'error: failed to snapshot Hyprland fragment configuration' >&2
            return 1
        fi
    fi

    overcrow_hypr_publish_error=
    if ! "$overcrow_hypr_install_program" -m 0644 \
            "$overcrow_rendered" "${overcrow_fragment}.new" || \
        ! "$overcrow_hypr_mv_program" -f -- \
            "${overcrow_fragment}.new" "$overcrow_fragment" || \
        ! "$overcrow_hypr_mv_program" -f -- "$overcrow_main_new" "$overcrow_main"; then
        overcrow_hypr_publish_error='failed to publish Hyprland configuration'
    elif ! overcrow_hypr_cleanup_install_temporaries \
        "$overcrow_rendered" "$overcrow_main_new" "${overcrow_fragment}.new"; then
        overcrow_hypr_publish_error="failed to clean Hyprland publication temporaries: $overcrow_hypr_cleanup_error"
    fi
    if [ -n "$overcrow_hypr_publish_error" ]; then
        if overcrow_hypr_restore_install \
            "$overcrow_main" "$overcrow_main_copy" \
            "$overcrow_fragment" "$overcrow_fragment_copy" \
            "$overcrow_fragment_existed"; then
            overcrow_hypr_cleanup_install_temporaries \
                "$overcrow_rendered" "$overcrow_main_new" \
                "${overcrow_fragment}.new" "$overcrow_main_copy" \
                "$overcrow_fragment_copy" || true
            printf '%s\n' "error: $overcrow_hypr_publish_error; restored previous files" >&2
        else
            overcrow_hypr_cleanup_install_temporaries \
                "$overcrow_rendered" "$overcrow_main_new" \
                "${overcrow_fragment}.new" || true
            printf '%s\n' "error: $overcrow_hypr_publish_error; rollback failed: $overcrow_hypr_restore_error; rollback copies preserved: $overcrow_main_copy $overcrow_fragment_copy" >&2
        fi
        return 1
    fi

    overcrow_validation_failed=false
    if [ "$overcrow_live" = true ]; then
        if overcrow_hypr_run_bounded \
            "$overcrow_hyprctl_program" reload >/dev/null 2>&1; then
            :
        else
            overcrow_reload_status=$?
            if overcrow_hypr_status_is_fatal "$overcrow_reload_status"; then
                printf '%s\n' "error: bounded Hyprland reload could not be started; rollback copies preserved: $overcrow_main_copy $overcrow_fragment_copy" >&2
                return 125
            fi
            overcrow_validation_failed=true
        fi
        if [ "$overcrow_validation_failed" = false ]; then
            if overcrow_errors_after=$(overcrow_hypr_run_bounded \
                "$overcrow_hyprctl_program" configerrors 2>/dev/null); then
                if [ "$overcrow_errors_after" != "$overcrow_errors_before" ] && \
                    [ "$overcrow_errors_after" != 'no errors' ]; then
                    overcrow_validation_failed=true
                fi
            else
                overcrow_errors_status=$?
                if overcrow_hypr_status_is_fatal "$overcrow_errors_status"; then
                    printf '%s\n' "error: bounded Hyprland validation could not be started; rollback copies preserved: $overcrow_main_copy $overcrow_fragment_copy" >&2
                    return 125
                fi
                overcrow_validation_failed=true
            fi
        fi
    fi

    if [ "$overcrow_validation_failed" = true ]; then
        if overcrow_hypr_restore_install \
            "$overcrow_main" "$overcrow_main_copy" \
            "$overcrow_fragment" "$overcrow_fragment_copy" \
            "$overcrow_fragment_existed"; then
            if overcrow_hypr_run_bounded \
                "$overcrow_hyprctl_program" reload >/dev/null 2>&1; then
                :
            else
                overcrow_restore_reload_status=$?
                if overcrow_hypr_status_is_fatal "$overcrow_restore_reload_status"; then
                    printf '%s\n' "error: bounded Hyprland reload could not be started after rollback; rollback copies preserved: $overcrow_main_copy $overcrow_fragment_copy" >&2
                    return 125
                fi
            fi
            overcrow_hypr_cleanup_install_temporaries \
                "$overcrow_main_copy" "$overcrow_fragment_copy" || true
            printf '%s\n' 'error: OverCrow introduced a Hyprland configuration error; restored previous files' >&2
        else
            printf '%s\n' "error: OverCrow introduced a Hyprland configuration error; rollback failed: $overcrow_hypr_restore_error; rollback copies preserved: $overcrow_main_copy $overcrow_fragment_copy" >&2
        fi
        return 1
    fi

    if ! overcrow_hypr_cleanup_install_temporaries \
        "$overcrow_main_copy" "$overcrow_fragment_copy"; then
        printf '%s\n' "error: Hyprland integration succeeded but rollback temporary cleanup failed: $overcrow_hypr_cleanup_error" >&2
        return 1
    fi
    return 0
}

hyprland_config_ready() {
    overcrow_config_home=$1
    overcrow_template_dir=$2

    overcrow_hypr_system_programs_ready || return 1
    if ! overcrow_hypr_safe_path "$overcrow_config_home" || \
        ! overcrow_hypr_safe_path "$overcrow_template_dir"; then
        return 1
    fi
    overcrow_hypr_dir="$overcrow_config_home/hypr"
    if ! overcrow_hypr_direct_directory "$overcrow_config_home" || \
        ! overcrow_hypr_direct_directory "$overcrow_hypr_dir"; then
        return 1
    fi

    overcrow_lua_main="$overcrow_hypr_dir/hyprland.lua"
    overcrow_conf_main="$overcrow_hypr_dir/hyprland.conf"
    if [ -f "$overcrow_lua_main" ]; then
        overcrow_main=$overcrow_lua_main
        overcrow_fragment="$overcrow_hypr_dir/overcrow.lua"
        overcrow_template="$overcrow_template_dir/overcrow.lua.in"
        overcrow_begin='-- BEGIN OVERCROW MANAGED'
        overcrow_end='-- END OVERCROW MANAGED'
        overcrow_link='require("overcrow")'
        overcrow_signature='-- Managed by OverCrow. Changes are replaced by the installer.'
    elif [ -f "$overcrow_conf_main" ]; then
        overcrow_main=$overcrow_conf_main
        overcrow_fragment="$overcrow_hypr_dir/overcrow.conf"
        overcrow_template="$overcrow_template_dir/overcrow.conf.in"
        overcrow_begin='# BEGIN OVERCROW MANAGED'
        overcrow_end='# END OVERCROW MANAGED'
        overcrow_link="source = $overcrow_fragment"
        overcrow_signature='# Managed by OverCrow. Changes are replaced by the installer.'
    else
        return 1
    fi

    [ -f "$overcrow_template" ] && [ ! -L "$overcrow_template" ] || return 1
    [ -f "$overcrow_fragment" ] && [ ! -L "$overcrow_fragment" ] || return 1
    overcrow_hypr_user_owned "$overcrow_main" || return 1
    overcrow_hypr_user_owned "$overcrow_fragment" || return 1
    [ "$("$overcrow_hypr_sed_program" -n \
        '1p' "$overcrow_fragment" 2>/dev/null)" = "$overcrow_signature" ] || return 1
    "$overcrow_hypr_cmp_program" -s "$overcrow_template" "$overcrow_fragment" || return 1

    overcrow_hypr_has_exact_block \
        "$overcrow_main" "$overcrow_begin" "$overcrow_link" "$overcrow_end"
}

overcrow_hypr_remove_from_main() {
    overcrow_remove_main=$1
    overcrow_remove_begin=$2
    overcrow_remove_end=$3
    [ -f "$overcrow_remove_main" ] || return 0
    if ! overcrow_hypr_user_owned "$overcrow_remove_main"; then
        printf '%s\n' "error: refusing to edit root-owned $overcrow_remove_main" >&2
        return 1
    fi
    overcrow_remove_parent=$("$overcrow_hypr_dirname_program" -- \
        "$overcrow_remove_main") || return 1
    overcrow_remove_tmp=$("$overcrow_hypr_mktemp_program" \
        "$overcrow_remove_parent/.overcrow-remove.XXXXXX") || return 1
    if ! overcrow_hypr_strip_block \
        "$overcrow_remove_main" "$overcrow_remove_tmp" "$overcrow_remove_begin" "$overcrow_remove_end"; then
        "$overcrow_hypr_rm_program" -f -- "$overcrow_remove_tmp"
        return 1
    fi
    if "$overcrow_hypr_cmp_program" -s \
        "$overcrow_remove_main" "$overcrow_remove_tmp"; then
        "$overcrow_hypr_rm_program" -f -- "$overcrow_remove_tmp"
        return 0
    fi
    overcrow_hypr_backup "$overcrow_remove_main" || {
        "$overcrow_hypr_rm_program" -f -- "$overcrow_remove_tmp"
        return 1
    }
    "$overcrow_hypr_chmod_program" --reference="$overcrow_remove_main" \
        "$overcrow_remove_tmp" || return 1
    "$overcrow_hypr_mv_program" -f -- "$overcrow_remove_tmp" "$overcrow_remove_main"
}

remove_hyprland_config() {
    overcrow_config_home=$1
    if ! overcrow_hypr_safe_path "$overcrow_config_home"; then
        printf '%s\n' 'error: Hyprland config path contains unsupported characters' >&2
        return 1
    fi
    if ! overcrow_hypr_system_programs_ready; then
        printf '%s\n' 'error: trusted system programs are unavailable' >&2
        return 1
    fi
    overcrow_hyprctl_available=false
    if overcrow_hyprctl_ready; then
        overcrow_hyprctl_available=true
    else
        overcrow_hyprctl_status=$?
        if [ "$overcrow_hyprctl_status" -eq 2 ]; then
            printf '%s\n' 'error: fixed Hyprland command is unsafe' >&2
            return 1
        fi
    fi
    overcrow_hypr_dir="$overcrow_config_home/hypr"
    if [ ! -e "$overcrow_hypr_dir" ] && [ ! -L "$overcrow_hypr_dir" ]; then
        return 0
    fi
    if ! overcrow_hypr_direct_directory "$overcrow_config_home" || \
        ! overcrow_hypr_direct_directory "$overcrow_hypr_dir"; then
        printf '%s\n' 'error: refusing a symlinked or foreign-owned Hyprland configuration directory' >&2
        return 1
    fi
    overcrow_hypr_remove_from_main "$overcrow_hypr_dir/hyprland.conf" \
        '# BEGIN OVERCROW MANAGED' '# END OVERCROW MANAGED' || return 1
    overcrow_hypr_remove_from_main "$overcrow_hypr_dir/hyprland.lua" \
        '-- BEGIN OVERCROW MANAGED' '-- END OVERCROW MANAGED' || return 1

    for overcrow_fragment in "$overcrow_hypr_dir/overcrow.conf" "$overcrow_hypr_dir/overcrow.lua"; do
        [ -e "$overcrow_fragment" ] || continue
        if ! overcrow_hypr_user_owned "$overcrow_fragment"; then
            printf '%s\n' "error: refusing to remove root-owned $overcrow_fragment" >&2
            return 1
        fi
        case $overcrow_fragment in
            *.conf) overcrow_signature='# Managed by OverCrow. Changes are replaced by the installer.' ;;
            *.lua) overcrow_signature='-- Managed by OverCrow. Changes are replaced by the installer.' ;;
        esac
        if "$overcrow_hypr_grep_program" -Fqx \
            "$overcrow_signature" "$overcrow_fragment"; then
            overcrow_hypr_backup "$overcrow_fragment" || return 1
            "$overcrow_hypr_rm_program" -f -- "$overcrow_fragment"
        else
            printf '%s\n' "error: refusing to remove unmanaged $overcrow_fragment" >&2
            return 1
        fi
    done

    if [ "$overcrow_hyprctl_available" = true ] && \
        overcrow_hypr_run_bounded \
            "$overcrow_hyprctl_program" configerrors >/dev/null 2>&1; then
        overcrow_hypr_run_bounded \
            "$overcrow_hyprctl_program" reload >/dev/null 2>&1 || true
    fi
}
