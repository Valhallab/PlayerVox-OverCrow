#!/bin/sh

overcrow_version_is_valid() {
    test "$#" -eq 1 || return 1
    printf '%s\n' "$1" | LC_ALL=C grep -Eq \
        '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
}

overcrow_arch_version() {
    test "$#" -eq 1 || return 1
    overcrow_version_is_valid "$1" || return 1

    case $1 in
        *-*)
            base=${1%%-*}
            prerelease=${1#*-}
            normalized=$(printf '%s\n' "$prerelease" | LC_ALL=C tr -d '.-')
            test -n "$normalized" || return 1
            printf '%s%s\n' "$base" "$normalized"
            ;;
        *)
            printf '%s\n' "$1"
            ;;
    esac
}

overcrow_rpm_version() {
    test "$#" -eq 1 || return 1
    overcrow_version_is_valid "$1" || return 1

    case $1 in
        *-*)
            base=${1%%-*}
            prerelease=${1#*-}
            normalized=$(printf '%s\n' "$prerelease" | LC_ALL=C tr '-' '_')
            test -n "$normalized" || return 1
            printf '%s~%s\n' "$base" "$normalized"
            ;;
        *)
            printf '%s\n' "$1"
            ;;
    esac
}

overcrow_rpm_artifact_version() {
    test "$#" -eq 1 || return 1
    rpm_version=$(overcrow_rpm_version "$1") || return 1
    printf '%s\n' "$rpm_version" | LC_ALL=C tr '~' '.'
}

overcrow_deb_upstream_version() {
    test "$#" -eq 1 || return 1
    overcrow_version_is_valid "$1" || return 1

    case $1 in
        *-*)
            base=${1%%-*}
            prerelease=${1#*-}
            normalized=$(printf '%s\n' "$prerelease" | LC_ALL=C tr '-' '.')
            test -n "$normalized" || return 1
            printf '%s~%s\n' "$base" "$normalized"
            ;;
        *)
            printf '%s\n' "$1"
            ;;
    esac
}

overcrow_deb_package_version() {
    test "$#" -eq 1 || return 1
    deb_upstream_version=$(overcrow_deb_upstream_version "$1") || return 1
    printf '%s-1\n' "$deb_upstream_version"
}

overcrow_ppa_upstream_version() {
    test "$#" -eq 2 || return 1
    deb_upstream_version=$(overcrow_deb_upstream_version "$1") || return 1

    case $2 in
        0|0[0-9]*|''|*[!0-9]*) return 1 ;;
    esac

    printf '%s+ppa%s\n' "$deb_upstream_version" "$2"
}

overcrow_ppa_package_version() {
    test "$#" -eq 2 || return 1
    ppa_upstream_version=$(overcrow_ppa_upstream_version "$1" "$2") ||
        return 1

    printf '%s-1~noble1\n' "$ppa_upstream_version"
}
