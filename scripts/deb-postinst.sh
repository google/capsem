#!/bin/bash
# deb-postinst.sh -- Post-install script for the Capsem .deb package.
#
# Runs as root after dpkg installs the package. Creates the per-user
# ~/.capsem layout, registers the systemd user unit, and runs setup.
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
    echo "capsem: could not determine installing user, skipping per-user setup"
    echo "capsem: run 'capsem setup' manually to complete installation"
    exit 0
fi

USER_HOME=$(eval echo "~$TARGET_USER")
CAPSEM_DIR="$USER_HOME/.capsem"

# Create user-level directory layout
mkdir -p "$CAPSEM_DIR/bin" "$CAPSEM_DIR/assets" "$CAPSEM_DIR/run"

# Symlink system binaries into user dir
for bin in capsem capsem-service capsem-process capsem-mcp capsem-gateway capsem-tray; do
    if [ -f "/usr/bin/$bin" ]; then
        ln -sf "/usr/bin/$bin" "$CAPSEM_DIR/bin/$bin"
    fi
done

# Fix ownership
chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$CAPSEM_DIR"

# Register systemd user unit and run setup (as the target user)
if command -v systemctl >/dev/null 2>&1; then
    su "$TARGET_USER" -c "$CAPSEM_DIR/bin/capsem service install" 2>/dev/null || true
fi
su "$TARGET_USER" -c "$CAPSEM_DIR/bin/capsem setup --non-interactive --accept-detected" 2>/dev/null || true

exit 0
