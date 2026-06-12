#!/bin/bash
# deb-postinst.sh -- Post-install script for the Capsem .deb package.
#
# Runs as root after dpkg installs the package. Creates the per-user
# ~/.capsem layout and registers the systemd user unit.
#
# The .deb installs companion binaries to /usr/bin/. This script
# symlinks them into ~/.capsem/bin/ for the user who installed.
set -euo pipefail

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
mkdir -p "$CAPSEM_DIR/bin" "$CAPSEM_DIR/assets" "$CAPSEM_DIR/run" "$CAPSEM_DIR/logs"
touch "$INSTALL_LOG" "$INSTALL_RUN_LOG"
printf '%s\n' "$INSTALL_RUN_ID" > "$INSTALL_RUN_FILE"
ln -sf "$(basename "$INSTALL_RUN_LOG")" "$CAPSEM_DIR/logs/install-latest.log"
chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$CAPSEM_DIR/logs"
exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1
echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=start user=$TARGET_USER install_run_id=$INSTALL_RUN_ID install_run_log=$INSTALL_RUN_LOG"

# Copy the package-selected manifest and provenance. VM asset payloads are
# external to the package and are reconciled by the service from this manifest.
if [ -f "/usr/share/capsem/assets/manifest.json" ]; then
    install -m 0644 /usr/share/capsem/assets/manifest.json "$CAPSEM_DIR/assets/manifest.json"
    if [ -f "/usr/share/capsem/assets/manifest-origin.json" ]; then
        install -m 0644 /usr/share/capsem/assets/manifest-origin.json "$CAPSEM_DIR/assets/manifest-origin.json"
    fi
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=manifest_copied"
fi

if [ -d "/usr/share/capsem/profiles" ]; then
    rm -rf "$CAPSEM_DIR/profiles"
    mkdir -p "$CAPSEM_DIR/profiles"
    cp -R /usr/share/capsem/profiles/. "$CAPSEM_DIR/profiles/" 2>/dev/null || true
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=profiles_copied"
fi

# Symlink system binaries into user dir
for bin in capsem capsem-service capsem-process capsem-tui capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray capsem-admin; do
    if [ -f "/usr/bin/$bin" ]; then
        ln -sf "/usr/bin/$bin" "$CAPSEM_DIR/bin/$bin"
    fi
done

# Fix ownership
chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$CAPSEM_DIR"

# Register systemd user unit as the target user.
# XDG_RUNTIME_DIR is required for systemctl --user; su drops it.
TARGET_UID=$(id -u "$TARGET_USER")
XDG_DIR="/run/user/$TARGET_UID"
if command -v systemctl >/dev/null 2>&1; then
    su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR $CAPSEM_DIR/bin/capsem install" 2>/dev/null || true
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=service_install_invoked"
fi

echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') phase=deb-postinst event=complete"
exit 0
