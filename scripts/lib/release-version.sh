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
