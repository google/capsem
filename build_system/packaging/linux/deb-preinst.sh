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
    source "$(dirname "$0")/../shared/install-diagnostics"
fi
if ! declare -F capsem_install_runs_inside_service >/dev/null; then
    source "$(dirname "$0")/../shared/service-owned-update"
fi
if ! declare -F capsem_retire_native_cohort >/dev/null; then
    source "$(dirname "$0")/../shared/retire-cohort"
fi

if [ -n "${SUDO_USER:-}" ]; then
    TARGET_USER="$SUDO_USER"
elif [ -n "${USER:-}" ] && [ "$USER" != "root" ]; then
    TARGET_USER="$USER"
else
    TARGET_USER=""
fi

if [ -z "$TARGET_USER" ]; then
    echo "capsem: no installing user supplied; run the package install with sudo from the target account" >&2
    exit 1
fi
case "$TARGET_USER" in
    root|*[!A-Za-z0-9._-]*) echo "capsem: invalid installing user" >&2; exit 1 ;;
esac
TARGET_UID=$(id -u "$TARGET_USER")
if [ "$TARGET_UID" -eq 0 ]; then
    echo "capsem: installing user must be non-root" >&2
    exit 1
fi

USER_HOME=$(getent passwd "$TARGET_USER" | cut -d: -f6)
case "$USER_HOME" in
    /*) ;;
    *) echo "capsem: installing user has no absolute home" >&2; exit 1 ;;
esac
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
CAPSEM_INSTALL_FAILURE_AS_USER=1
capsem_install_enable_failure_trap
su "$TARGET_USER" -c "CAPSEM_INSTALL_RUN_ID=$INSTALL_RUN_ID /bin/bash -s" <<'CAPSEM_USER'
set -euo pipefail
umask 077
mkdir -p "$HOME/.capsem/logs"
touch "$HOME/.capsem/logs/install.log" "$HOME/.capsem/logs/install-$CAPSEM_INSTALL_RUN_ID.log"
printf '%s\n' "$CAPSEM_INSTALL_RUN_ID" > "$HOME/.capsem/logs/install-current-run"
ln -sf "install-$CAPSEM_INSTALL_RUN_ID.log" "$HOME/.capsem/logs/install-latest.log"
rm -f "$HOME/.capsem/logs/install-failure.txt"
CAPSEM_USER
exec > >(su "$TARGET_USER" -c "tee -a \"\$HOME/.capsem/logs/install.log\" \"\$HOME/.capsem/logs/install-$INSTALL_RUN_ID.log\"") 2>&1
echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=start user=$TARGET_USER install_run_id=$INSTALL_RUN_ID install_run_log=$INSTALL_RUN_LOG"

CAPSEM_INSTALL_PHASE="admit_existing_ledger"
if ! declare -F capsem_run_preinstall_ledger_admission >/dev/null; then
    echo 'capsem: package is missing its candidate preinstall ledger resource' >&2
    exit 1
fi
capsem_run_preinstall_ledger_admission "$TARGET_USER"

CAPSEM_INSTALL_PHASE="stop_existing_install"
XDG_DIR="/run/user/$TARGET_UID"
if capsem_install_runs_inside_service /proc/self/cgroup; then
    # The old producer remains live here: an absent HOST cannot safely transition.
    capsem_run_preinstall_ledger_admission "$TARGET_USER" --require-current-host
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=preserve_service_owned_update unit=capsem.service"
else
    # Match postinstall's sd_booted predicate. Merely having systemctl does
    # not mean PID1 is systemd. A booted manager must still verify quiescence;
    # unavailable user-manager/query state is never interpreted as absence.
    if [ -d /run/systemd/system ]; then
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=stop_systemd_user_service unit=capsem.service"
        capsem_stop_systemd_service "$TARGET_USER" "$XDG_DIR"
    fi
    capsem_retire_native_cohort "$CAPSEM_DIR" "$TARGET_UID" "$TARGET_USER"
    CAPSEM_INSTALL_PHASE="admit_drained_ledger"
    capsem_run_preinstall_ledger_admission "$TARGET_USER"
fi

echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-preinst event=complete"
CAPSEM_INSTALL_PHASE="complete"
exit 0
