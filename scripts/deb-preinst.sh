#!/bin/bash
# deb-preinst.sh -- Retire stale Capsem processes before .deb replacement.
#
# dpkg can replace /usr/bin payloads while the old service, gateway, tray, or
# process binaries keep running from old inodes. Stop the user unit first, then
# kill package-owned helpers before unpacking new binaries. A package update
# started by the old service must instead preserve that cohort until the old
# updater activates the manifest and requests its managed restart.
set -euo pipefail
if ! declare -F capsem_install_enable_failure_trap >/dev/null; then
    source "$(dirname "$0")/pkg-scripts/install-diagnostics"
fi
if ! declare -F capsem_install_runs_inside_service >/dev/null; then
    source "$(dirname "$0")/pkg-scripts/service-owned-update"
fi
if ! declare -F capsem_retire_native_cohort >/dev/null; then
    source "$(dirname "$0")/pkg-scripts/retire-cohort"
fi

if [ -n "${SUDO_USER:-}" ]; then
    TARGET_USER="$SUDO_USER"
elif [ -n "${USER:-}" ] && [ "$USER" != "root" ]; then
    TARGET_USER="$USER"
else
    TARGET_USER=$(getent passwd 1000 | cut -d: -f1 || true)
fi

if [ -z "${TARGET_USER:-}" ]; then
    echo "capsem: could not determine installing user, skipping pre-install shutdown"
    exit 0
fi

USER_HOME=$(eval echo "~$TARGET_USER")
CAPSEM_DIR="$USER_HOME/.capsem"
INSTALL_RUN_ID=$(date -u '+%Y%m%dT%H%M%SZ')
INSTALL_LOG="$CAPSEM_DIR/logs/install.log"
INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"
INSTALL_RUN_FILE="$CAPSEM_DIR/logs/install-current-run"

CAPSEM_INSTALL_PHASE="initialize_log"
CAPSEM_INSTALL_RUN_LOG="$INSTALL_RUN_LOG"
CAPSEM_INSTALL_FAILURE_FILE="$CAPSEM_DIR/logs/install-failure.txt"
CAPSEM_INSTALL_USER="$TARGET_USER"
CAPSEM_INSTALL_PRESENT_FAILURE=0
capsem_install_enable_failure_trap
rm -f "$CAPSEM_INSTALL_FAILURE_FILE"
mkdir -p "$CAPSEM_DIR/logs"
touch "$INSTALL_LOG" "$INSTALL_RUN_LOG"
printf '%s\n' "$INSTALL_RUN_ID" > "$INSTALL_RUN_FILE"
ln -sf "$(basename "$INSTALL_RUN_LOG")" "$CAPSEM_DIR/logs/install-latest.log"
chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$CAPSEM_DIR/logs" 2>/dev/null || true
exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1
echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=start user=$TARGET_USER install_run_id=$INSTALL_RUN_ID install_run_log=$INSTALL_RUN_LOG"

CAPSEM_INSTALL_PHASE="stop_existing_install"
TARGET_UID=$(id -u "$TARGET_USER")
XDG_DIR="/run/user/$TARGET_UID"
if capsem_install_runs_inside_service /proc/self/cgroup; then
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=preserve_service_owned_update unit=capsem.service"
else
    if command -v systemctl >/dev/null 2>&1 && [ -d "$XDG_DIR" ]; then
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=stop_systemd_user_service unit=capsem.service"
        su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR systemctl --user stop capsem.service" 2>/dev/null || true
    fi
    capsem_retire_native_cohort "$CAPSEM_DIR" "$TARGET_UID"
fi

echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=complete"
CAPSEM_INSTALL_PHASE="complete"
exit 0
