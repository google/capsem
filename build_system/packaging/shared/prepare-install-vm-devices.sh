#!/usr/bin/env bash
# Give the unprivileged installed-proof user only the numeric groups owning
# the VM devices passed through from this Docker/Colima host, then become PID 1.
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
    if runuser -u "$guest_user" -- test -r "$device" -a -w "$device"; then
        continue
    fi
    group_id=$(stat -c %g -- "$device")
    mode=$(stat -c %a -- "$device")
    group_mode=$(( (10#$mode / 10) % 10 ))
    if (( group_id == 0 || (group_mode & 6) != 6 )); then
        echo "VM device cannot be granted by a non-root read/write group: $device" >&2
        exit 2
    fi
    group_name=$(getent group "$group_id" | cut -d: -f1 || true)
    if [[ -z "$group_name" ]]; then
        group_name="capsem-vm-$group_id"
        groupadd --gid "$group_id" "$group_name"
    fi
    usermod --append --groups "$group_name" "$guest_user"
    runuser -u "$guest_user" -- test -r "$device" -a -w "$device"
done

exec "$systemd_command"
