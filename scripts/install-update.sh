#!/bin/sh
set -u
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

replace_app() {
    source_app=$1
    destination_app=$2
    case "$source_app" in /*.app) ;; *) return 1 ;; esac
    case "$destination_app" in /*.app) ;; *) return 1 ;; esac
    [ -d "$source_app" ] && [ ! -L "$source_app" ] && [ ! -L "$destination_app" ] || return 1
    new_app="${destination_app%.app}.pixclip-new-$$.app"
    old_app="${destination_app%.app}.pixclip-old-$$.app"
    [ ! -e "$new_app" ] && [ ! -e "$old_app" ] || return 1
    if ! ditto "$source_app" "$new_app"; then
        rm -rf -- "$new_app"
        return 1
    fi
    if ! codesign --verify --deep --strict "$new_app"; then
        rm -rf -- "$new_app"
        return 1
    fi
    if ! identifier=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$new_app/Contents/Info.plist"); then
        rm -rf -- "$new_app"
        return 1
    fi
    if [ "$identifier" != io.github.PeakNAtanon.PixClip ]; then
        rm -rf -- "$new_app"
        return 1
    fi
    if [ -e "$destination_app" ]; then
        if ! mv -- "$destination_app" "$old_app"; then
            rm -rf -- "$new_app"
            return 1
        fi
    fi
    if ! mv -- "$new_app" "$destination_app"; then
        if [ -e "$old_app" ]; then mv -- "$old_app" "$destination_app" || return 1; fi
        rm -rf -- "$new_app"
        return 1
    fi
    if [ -e "$old_app" ]; then rm -rf -- "$old_app" || true; fi
}

if [ "${1:-}" = --replace-app ]; then
    [ "$#" -eq 3 ] || exit 2
    replace_app "$2" "$3"
    exit $?
fi

[ "$#" -eq 7 ] || exit 2
parent=$1
kind=$2
package=$3
expected_hash=$4
original=$5
destination=$6
log=$7
restart_allowed=no
installed=no
mount_dir=
exec >>"$log" 2>&1

cleanup() {
    if [ -n "$mount_dir" ]; then
        hdiutil detach "$mount_dir" || true
        rmdir "$mount_dir" || true
    fi
}
trap cleanup EXIT

restart_app() {
    [ "$restart_allowed" = yes ] || return 0
    case "$kind" in
        dmg)
            if [ "$installed" = yes ]; then
                open -n "$destination"
            else
                case "$original" in
                    *.app/Contents/MacOS/*) open -n "${original%/Contents/MacOS/*}" ;;
                    *) "$original" --gui >>"$log" 2>&1 & ;;
                esac
            fi ;;
        *)
            executable=$original
            if [ "$installed" = yes ]; then executable=$destination; fi
            "$executable" --gui >>"$log" 2>&1 & ;;
    esac
}

fail() {
    printf '%s\nLog: %s\n' "$1" "$log" >"$log.failed"
    restart_app
    exit 1
}

case "$kind" in deb|rpm|arch|dmg) ;; *) fail 'Unsupported update package.' ;; esac
attempts=120
while :; do
    if ! kill -0 "$parent" 2>/dev/null; then
        break
    fi
    [ "$attempts" -gt 0 ] || fail 'PixClip did not close. The update was not installed.'
    attempts=$((attempts - 1))
    sleep 1
done
restart_allowed=yes

if [ "$kind" = dmg ]; then
    actual_hash=$(shasum -a 256 "$package") || fail 'Could not read the downloaded update.'
else
    actual_hash=$(sha256sum "$package") || fail 'Could not read the downloaded update.'
fi
actual_hash=${actual_hash%% *}
[ "$actual_hash" = "$expected_hash" ] || fail 'Installer checksum changed. The update was cancelled.'

as_admin() {
    if [ "$(id -u)" -eq 0 ]; then "$@"; else pkexec "$@"; fi
}

install_update() {
    case "$kind" in
        deb) as_admin apt-get install -y "$package" ;;
        arch) as_admin pacman -U --noconfirm "$package" ;;
        rpm)
            if command -v dnf >/dev/null 2>&1; then
                as_admin dnf install -y "$package"
            elif command -v zypper >/dev/null 2>&1; then
                as_admin zypper --non-interactive install --allow-unsigned-rpm "$package"
            else return 1; fi ;;
        dmg)
            mount_dir=$(mktemp -d "${TMPDIR:-/tmp}/pixclip-update.XXXXXX") || return 1
            hdiutil attach "$package" -nobrowse -readonly -mountpoint "$mount_dir" || return 1
            mkdir -p "$(dirname "$destination")" || true
            if [ -w "$(dirname "$destination")" ] && { [ ! -e "$destination" ] || [ -w "$destination" ]; }; then
                replace_app "$mount_dir/PixClip.app" "$destination"
            else
                osascript - "$0" "$mount_dir/PixClip.app" "$destination" <<'APPLESCRIPT'
on run argv
    set commandText to "/bin/sh " & quoted form of (item 1 of argv) & " --replace-app " & quoted form of (item 2 of argv) & " " & quoted form of (item 3 of argv)
    do shell script commandText with administrator privileges
end run
APPLESCRIPT
            fi ;;
    esac
}

if install_update; then
    installed=yes
    rm -f -- "$log.failed"
    restart_app
else
    fail 'The update installer failed or permission was declined. PixClip will try to reopen. Check the installation log.'
fi
