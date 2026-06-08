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

# Create user-level directory layout
mkdir -p "$CAPSEM_DIR/bin" "$CAPSEM_DIR/assets" "$CAPSEM_DIR/run"

# Copy package-provided assets, if present. Local dev packages include the
# current-arch payload; release packages may provide only a manifest and let
# the service reconcile assets independently.
if [ -d "/usr/share/capsem/assets" ]; then
    cp -R /usr/share/capsem/assets/. "$CAPSEM_DIR/assets/" 2>/dev/null || true
fi

# Symlink system binaries into user dir
for bin in capsem capsem-service capsem-process capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray capsem-admin; do
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
fi

exit 0
