#!/bin/bash
# deb-preinst.sh -- Stop stale Capsem user services before .deb replacement.
#
# dpkg can replace /usr/bin payloads while the old service, gateway, tray, or
# process binaries keep running from old inodes. Stop the user unit first, then
# kill any package-owned helpers by exact installed paths before unpacking the
# new binaries.
set -euo pipefail
if ! declare -F capsem_install_enable_failure_trap >/dev/null; then
    source "$(dirname "$0")/pkg-scripts/install-diagnostics"
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
if command -v systemctl >/dev/null 2>&1 && [ -d "$XDG_DIR" ]; then
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=stop_systemd_user_service unit=capsem.service"
    su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR systemctl --user stop capsem.service" 2>/dev/null || true
fi

for name in capsem-service capsem-gateway capsem-tray capsem-process capsem-mcp-aggregator capsem-mcp-builtin; do
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=kill_process name=$name"
    pkill -9 -f "$CAPSEM_DIR/bin/$name" 2>/dev/null || true
    pkill -9 -f "/usr/bin/$name" 2>/dev/null || true
done

echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=complete"
CAPSEM_INSTALL_PHASE="complete"
exit 0
