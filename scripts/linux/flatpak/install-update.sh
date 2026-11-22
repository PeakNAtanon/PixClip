#!/bin/sh
set -eu
PATH=/usr/bin:/bin
export PATH

[ "$#" -eq 4 ] || exit 2
package=$1
expected_hash=$2
app_path=$3
log=$4
app_ref=app/io.github.PeakNAtanon.PixClip/x86_64/stable

# Restore only host desktop session variables used by Flatpak and the app window.
XDG_RUNTIME_DIR=/run/user/$(id -u)
DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus
export XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS
session_environment=$(systemctl --user show-environment 2>/dev/null) || session_environment=
while IFS= read -r entry; do
    case "$entry" in
        DISPLAY=*|WAYLAND_DISPLAY=*|XAUTHORITY=*|XDG_DATA_HOME=*) export "$entry" ;;
    esac
done <<EOF
$session_environment
EOF

scope=
for candidate in --user --system; do
    if location=$(flatpak info "$candidate" --show-location "$app_ref" 2>/dev/null) &&
       [ "$location/files" = "$app_path" ]; then
        scope=$candidate
        break
    fi
done
[ -n "$scope" ] || exit 1
[ -r "$package" ] || exit 1
exec 3>"$log"

# Keep the original app open until the host confirms it can run this update.
printf 'ready\n'
IFS= read -r approval
[ "$approval" = install ] || exit 1
# PixClip holds this pipe open until its process exits and releases the queue lock.
if IFS= read -r extra; then exit 1; fi
exec >&3 2>&1 3>&-

restart() {
    flatpak run "$scope" "$app_ref" </dev/null >>"$log" 2>&1 &
}

fail() {
    printf '%s\nLog: %s\n' "$1" "$log" >"$log.failed"
    restart
    exit 1
}

actual_hash=$(sha256sum -- "$package") || fail 'Could not read the downloaded Flatpak bundle.'
[ "${actual_hash%% *}" = "$expected_hash" ] || fail 'The Flatpak bundle checksum changed. Update cancelled.'
if ! flatpak install "$scope" --bundle --or-update --noninteractive --assumeyes "$package"; then
    fail 'Flatpak could not install the update. Check the installation log.'
fi
rm -f -- "$log.failed"
restart
