#!/usr/bin/env bash
# Start the proof's user manager before the package grants VM-device access.
# This reproduces an installed user's stale supplementary-group credentials.
set -euo pipefail

if (( $# < 2 )); then
    echo "usage: $0 GUEST_USER SYSTEMD_COMMAND [DEVICE ...]" >&2
    exit 2
fi

guest_user=$1
systemd_command=$2
shift 2

id "$guest_user" >/dev/null
test -x "$systemd_command"

for device in "$@"; do
    if [[ ! -c "$device" ]]; then
        echo "VM device is not a character device: $device" >&2
        exit 2
    fi
done

exec "$systemd_command"
