#!/bin/bash
# deb-postinst.sh -- Post-install script for the Capsem .deb package.
#
# Runs as root after dpkg installs the package. Creates the per-user
# ~/.capsem layout and registers the systemd user unit.
#
# The .deb installs companion binaries to /usr/bin/. This script
# symlinks them into ~/.capsem/bin/ for the user who installed.
set -euo pipefail
if ! declare -F capsem_install_enable_failure_trap >/dev/null; then
    source "$(dirname "$0")/pkg-scripts/install-diagnostics"
fi

# Determine the real user (not root from sudo)
if [ -n "${SUDO_USER:-}" ]; then
    TARGET_USER="$SUDO_USER"
elif [ -n "${USER:-}" ] && [ "$USER" != "root" ]; then
    TARGET_USER="$USER"
else
    # Fall back: first non-root user with a home directory
    TARGET_USER=$(getent passwd 1000 | cut -d: -f1 || true)
fi

if [ -z "$TARGET_USER" ]; then
    echo "capsem: could not determine installing user, skipping per-user install"
    exit 0
fi

USER_HOME=$(eval echo "~$TARGET_USER")
CAPSEM_DIR="$USER_HOME/.capsem"
INSTALL_LOG="$CAPSEM_DIR/logs/install.log"
INSTALL_RUN_ID=$(date -u '+%Y%m%dT%H%M%SZ')
INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"
INSTALL_RUN_FILE="$CAPSEM_DIR/logs/install-current-run"

# Create user-level directory layout
CAPSEM_INSTALL_PHASE="initialize_log"
CAPSEM_INSTALL_RUN_LOG="$INSTALL_RUN_LOG"
CAPSEM_INSTALL_FAILURE_FILE="$CAPSEM_DIR/logs/install-failure.txt"
CAPSEM_INSTALL_USER="$TARGET_USER"
CAPSEM_INSTALL_PRESENT_FAILURE=0
capsem_install_enable_failure_trap
rm -f "$CAPSEM_INSTALL_FAILURE_FILE"
mkdir -p "$CAPSEM_DIR/bin" "$CAPSEM_DIR/assets" "$CAPSEM_DIR/run" "$CAPSEM_DIR/logs"
touch "$INSTALL_LOG" "$INSTALL_RUN_LOG"
printf '%s\n' "$INSTALL_RUN_ID" > "$INSTALL_RUN_FILE"
ln -sf "$(basename "$INSTALL_RUN_LOG")" "$CAPSEM_DIR/logs/install-latest.log"
chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$CAPSEM_DIR/logs"
exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1
echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=start user=$TARGET_USER install_run_id=$INSTALL_RUN_ID install_run_log=$INSTALL_RUN_LOG"
CAPSEM_INSTALL_PHASE="prepare_layout"
retired_user_config="user"".toml"
rm -f "$CAPSEM_DIR/$retired_user_config" "$CAPSEM_DIR/service.toml"
echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=retired_config_removed"
rm -rf "$CAPSEM_DIR/bin/capsem-admin-python"
echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=retired_python_admin_bundle_removed"
rm -rf "$CAPSEM_DIR"/update-check*
rm -f "$CAPSEM_DIR/assets"/manifest-*origin*.json
echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=obsolete_manifest_state_removed"

CAPSEM_INSTALL_PHASE="install_manifest_provenance"
if [ ! -f "/usr/share/capsem/assets/manifest-metadata.json" ]; then
    echo "capsem: packaged manifest-metadata.json missing" >&2
    exit 1
fi
install -m 0644 /usr/share/capsem/assets/manifest-metadata.json "$CAPSEM_DIR/assets/manifest-metadata.json"
MANIFEST_METADATA=$(tr '\n' ' ' < "$CAPSEM_DIR/assets/manifest-metadata.json")
echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=manifest_metadata $MANIFEST_METADATA"
MANIFEST_SOURCE=$(sed -n 's/.*"manifest_url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$CAPSEM_DIR/assets/manifest-metadata.json" | head -n 1)
if [ -z "$MANIFEST_SOURCE" ]; then
    echo "capsem: packaged manifest-metadata.json has no manifest_url" >&2
    exit 1
fi
echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=manifest_source source=$MANIFEST_SOURCE"

CAPSEM_INSTALL_PHASE="install_profiles"
if [ -d "/usr/share/capsem/profiles" ]; then
    rm -rf "$CAPSEM_DIR/profiles"
    mkdir -p "$CAPSEM_DIR/profiles"
    cp -R /usr/share/capsem/profiles/. "$CAPSEM_DIR/profiles/" 2>/dev/null || true
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=profiles_copied"
fi

# Symlink system binaries into user dir
CAPSEM_INSTALL_PHASE="link_binaries"
for bin in capsem capsem-service capsem-process capsem-tui capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray capsem-admin; do
    if [ ! -f "/usr/bin/$bin" ]; then
        echo "capsem: packaged binary missing: /usr/bin/$bin" >&2
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=binary_missing bin=$bin src=/usr/bin/$bin"
        exit 1
    fi
    ln -sf "/usr/bin/$bin" "$CAPSEM_DIR/bin/$bin"
done

# Fix ownership
chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$CAPSEM_DIR"

CAPSEM_INSTALL_PHASE="hydrate_assets"
if [ -x "$CAPSEM_DIR/bin/capsem" ]; then
    if ! su "$TARGET_USER" -c "CAPSEM_HOME=\"$CAPSEM_DIR\" CAPSEM_RUN_DIR=\"$CAPSEM_DIR/run\" \"$CAPSEM_DIR/bin/capsem\" update --assets --manifest \"$MANIFEST_SOURCE\""; then
        echo "capsem: asset hydration failed" >&2
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=asset_hydration_failed"
        exit 1
    fi
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=assets_hydrated"
fi

case "$MANIFEST_SOURCE" in
    http://*|https://*)
        CAPSEM_INSTALL_PHASE="refresh_update_status"
        if ! su "$TARGET_USER" -c "CAPSEM_HOME=\"$CAPSEM_DIR\" CAPSEM_RUN_DIR=\"$CAPSEM_DIR/run\" \"$CAPSEM_DIR/bin/capsem\" update --check"; then
            echo "capsem: update status refresh failed" >&2
            echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=update_status_refresh_failed source=$MANIFEST_SOURCE"
            exit 1
        fi
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=update_status_refreshed source=$MANIFEST_SOURCE"
        ;;
    *)
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=update_status_refresh_skipped source=$MANIFEST_SOURCE reason=non_http_manifest"
        ;;
esac

# Register systemd user unit as the target user.
# XDG_RUNTIME_DIR is required for systemctl --user; su drops it.
CAPSEM_INSTALL_PHASE="register_service"
TARGET_UID=$(id -u "$TARGET_USER")
XDG_DIR="/run/user/$TARGET_UID"
if command -v systemctl >/dev/null 2>&1; then
    if ! su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR $CAPSEM_DIR/bin/capsem install" 2>/dev/null; then
        echo "capsem: service registration failed" >&2
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=service_registration_failed"
        exit 1
    fi
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=service_install_invoked"

    READY=0
    STATUS_OUTPUT=""
    CAPSEM_INSTALL_PHASE="wait_for_service"
    for attempt in $(seq 1 30); do
        STATUS_OUTPUT=$(su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR CAPSEM_HOME=$CAPSEM_DIR CAPSEM_RUN_DIR=$CAPSEM_DIR/run $CAPSEM_DIR/bin/capsem status" 2>/dev/null || true)
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=readiness_poll attempt=$attempt"
        PROFILE_COUNTS=$(echo "$STATUS_OUTPUT" | sed -n 's/^Profiles:[[:space:]]*\([0-9][0-9]*\)\/\([0-9][0-9]*\) ready.*/\1 \2/p' | head -n 1)
        READY_PROFILES=${PROFILE_COUNTS%% *}
        TOTAL_PROFILES=${PROFILE_COUNTS##* }
        if echo "$STATUS_OUTPUT" | grep -q "Service:   ok" \
            && echo "$STATUS_OUTPUT" | grep -q "Gateway:   ok" \
            && [ -n "$PROFILE_COUNTS" ] \
            && [ "$READY_PROFILES" = "$TOTAL_PROFILES" ] \
            && [ "$TOTAL_PROFILES" -gt 0 ]; then
            READY=1
            break
        fi
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=profiles_not_ready attempt=$attempt ready=${READY_PROFILES:-unknown} total=${TOTAL_PROFILES:-unknown}"
        sleep 1
    done
    if [ "$READY" -ne 1 ]; then
        echo "capsem: service is not ready after install" >&2
        echo "$STATUS_OUTPUT" >&2
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=service_diagnostics"
        su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR systemctl --user status capsem.service --no-pager -l" 2>&1 || true
        su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR journalctl --user-unit capsem.service --no-pager -n 100" 2>&1 || true
        echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=service_not_ready"
        exit 1
    fi
fi

echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=complete"
CAPSEM_INSTALL_PHASE="complete"
exit 0
