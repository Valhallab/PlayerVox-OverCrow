#!/bin/sh
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
destination=${1:?usage: stage.sh DESTINATION THIRD_PARTY_NOTICES}
notices=${2:?usage: stage.sh DESTINATION THIRD_PARTY_NOTICES}

case $destination in
    /*) ;;
    *) printf '%s\n' 'error: both paths must be absolute' >&2; exit 2 ;;
esac
case $notices in
    /*) ;;
    *) printf '%s\n' 'error: both paths must be absolute' >&2; exit 2 ;;
esac

destination_parent=$(dirname -- "$destination")
test -d "$destination_parent" || {
    printf '%s\n' "error: destination parent does not exist: $destination_parent" >&2
    exit 2
}
if test -e "$destination" || test -L "$destination"; then
    printf '%s\n' "error: destination already exists: $destination" >&2
    exit 2
fi
test -s "$notices" || {
    printf '%s\n' "error: third-party notices are missing or empty: $notices" >&2
    exit 2
}

case ${SOURCE_DATE_EPOCH:-} in
    '') ;;
    *[!0-9]*)
        printf '%s\n' 'error: SOURCE_DATE_EPOCH must be a non-negative integer' >&2
        exit 2
        ;;
esac

validate_release_binary() {
    binary_name=$1
    binary_path="$project_root/target/release/$binary_name"

    if ! elf_report=$(LC_ALL=C readelf --file-header --program-headers --wide \
            "$binary_path" 2>&1); then
        printf '%s\n' "error: cannot inspect release binary: $binary_name" >&2
        return 1
    fi
    if ! printf '%s\n' "$elf_report" | grep -Eq '^  Class:[[:space:]]+ELF64$' ||
            ! printf '%s\n' "$elf_report" |
                grep -Eq '^  Machine:[[:space:]]+Advanced Micro Devices X86-64$'; then
        printf '%s\n' "error: release binary is not ELF64 x86-64: $binary_name" >&2
        return 1
    fi
    if ! printf '%s\n' "$elf_report" |
            grep -Eq 'Requesting program interpreter: /(lib64|usr/lib)/ld-linux-x86-64\.so\.2]'; then
        printf '%s\n' "error: release binary has an unsupported GNU/Linux interpreter: $binary_name" >&2
        return 1
    fi
}

for binary in overcrow-control overcrow-core overcrow-hyprland overcrow-overlay overcrowctl; do
    validate_release_binary "$binary"
done

working=
cleanup_working() {
    if test -n "$working" &&
            { test -e "$working" || test -L "$working"; }; then
        rm -rf -- "$working"
    fi
}
handle_exit() {
    exit_status=$?
    trap - EXIT HUP INT TERM
    cleanup_working || exit_status=1
    exit "$exit_status"
}
handle_signal() {
    trap - EXIT HUP INT TERM
    cleanup_working || :
    exit 1
}
trap handle_exit EXIT
trap handle_signal HUP INT TERM

working=$(mktemp -d "$destination_parent/.overcrow-stage.XXXXXX")
chmod 0700 "$working"

install -d -m 0755 \
    "$working/usr/bin" \
    "$working/usr/lib/overcrow" \
    "$working/usr/lib/systemd/user" \
    "$working/usr/share/applications" \
    "$working/usr/share/metainfo" \
    "$working/usr/share/icons/hicolor/512x512/apps" \
    "$working/usr/share/licenses/overcrow" \
    "$working/usr/share/overcrow/integrations/hyprland" \
    "$working/usr/share/overcrow/integrations/kwin/contents/code"

for binary in overcrow-control overcrow-core overcrow-hyprland overcrow-overlay overcrowctl; do
    install -m 0755 "$project_root/target/release/$binary" "$working/usr/bin/$binary"
done

install -m 0755 "$project_root/scripts/integrate-user.sh" \
    "$working/usr/lib/overcrow/overcrow-integrate"
install -m 0644 "$project_root/scripts/lib/hyprland-config.sh" \
    "$working/usr/lib/overcrow/hyprland-config.sh"

for unit in overcrow-core overcrow-hyprland overcrow-overlay; do
    sed 's|@OVERCROW_BINDIR@|/usr/bin|g' \
        "$project_root/packaging/systemd/$unit.service.in" \
        > "$working/usr/lib/systemd/user/$unit.service"
    test "$(stat -c '%a' "$working/usr/lib/systemd/user/$unit.service")" = 644 || \
        chmod 0644 "$working/usr/lib/systemd/user/$unit.service"
done

install -m 0644 "$project_root/packaging/applications/com.playervox.OverCrow.desktop" \
    "$working/usr/share/applications/com.playervox.OverCrow.desktop"
install -m 0644 "$project_root/packaging/metainfo/com.playervox.OverCrow.metainfo.xml" \
    "$working/usr/share/metainfo/com.playervox.OverCrow.metainfo.xml"
install -m 0644 "$project_root/crates/overcrow-control-ui/src-tauri/icons/icon.png" \
    "$working/usr/share/icons/hicolor/512x512/apps/com.playervox.OverCrow.png"
install -m 0644 "$project_root/integrations/kwin/metadata.json" \
    "$working/usr/share/overcrow/integrations/kwin/metadata.json"
install -m 0644 "$project_root/integrations/kwin/contents/code/main.js" \
    "$working/usr/share/overcrow/integrations/kwin/contents/code/main.js"
install -m 0644 "$project_root/integrations/hyprland/overcrow.conf.in" \
    "$working/usr/share/overcrow/integrations/hyprland/overcrow.conf.in"
install -m 0644 "$project_root/integrations/hyprland/overcrow.lua.in" \
    "$working/usr/share/overcrow/integrations/hyprland/overcrow.lua.in"
install -m 0644 "$project_root/LICENSE" \
    "$working/usr/share/licenses/overcrow/LICENSE"
install -m 0644 "$project_root/NOTICE" \
    "$working/usr/share/licenses/overcrow/NOTICE"
install -m 0644 "$project_root/assets/branding/NotoSans-OFL.txt" \
    "$working/usr/share/licenses/overcrow/NotoSans-OFL.txt"
install -m 0644 "$notices" \
    "$working/usr/share/licenses/overcrow/THIRD_PARTY_LICENSES.md"
install -m 0644 "$project_root/TRADEMARKS.md" \
    "$working/usr/share/overcrow/TRADEMARKS.md"

scan_fixed_text() {
    forbidden_text=$1
    failure_message=$2

    if grep -R -a -q -F "$forbidden_text" "$working"; then
        printf '%s\n' "$failure_message" >&2
        return 1
    else
        scan_status=$?
    fi
    if test "$scan_status" -ne 1; then
        printf '%s\n' 'error: could not scan staged runtime' >&2
        return 1
    fi
}

scan_fixed_text '@OVERCROW_BINDIR@' \
    'error: staged runtime contains unresolved binary-directory token'
scan_fixed_text "$project_root" \
    'error: staged runtime contains private workspace path'

if test -n "${SOURCE_DATE_EPOCH:-}"; then
    if ! find "$working" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +; then
        printf '%s\n' 'error: could not apply SOURCE_DATE_EPOCH' >&2
        exit 1
    fi
fi

chmod 0755 "$working"

if test -e "$destination" || test -L "$destination"; then
    printf '%s\n' "error: destination appeared before publication: $destination" >&2
    exit 1
fi
if ! mv -T -n -- "$working" "$destination"; then
    printf '%s\n' "error: could not publish destination: $destination" >&2
    exit 1
fi
if test -e "$working" || test -L "$working"; then
    printf '%s\n' "error: destination appeared during publication: $destination" >&2
    exit 1
fi
working=
test -d "$destination" || {
    printf '%s\n' "error: publication did not create destination: $destination" >&2
    exit 1
}
