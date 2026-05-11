#!/bin/bash
# deb-postinst.sh -- Post-install script for the Capsem .deb package.
#
# Runs as root after dpkg installs the package. Creates the per-user
# ~/.capsem layout, registers the systemd user unit, and runs setup.
#
# The .deb installs companion binaries to /usr/bin/ and signed manifest files
# to /usr/share/capsem/assets/. This script symlinks binaries into
# ~/.capsem/bin/ and seeds the user's asset manifest.
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
PKG_SHARE="/usr/share/capsem"

seed_asset_manifests() {
    for asset in manifest.json manifest.json.minisig; do
        if [ -f "$PKG_SHARE/assets/$asset" ]; then
            install -m 0644 "$PKG_SHARE/assets/$asset" "$CAPSEM_DIR/assets/$asset"
        fi
    done
    if [ -f "$PKG_SHARE/assets/manifest-sign.dev.pub" ]; then
        install -m 0644 "$PKG_SHARE/assets/manifest-sign.dev.pub" \
            "$CAPSEM_DIR/assets/manifest-sign.dev.pub"
    fi
    if [ -f "$PKG_SHARE/assets/manifest-sign.dev.pub" ] \
        && [ ! -f "$CAPSEM_DIR/assets/manifest-sign.dev.pub" ]; then
        echo "capsem: manifest-sign.dev.pub failed to install" >&2
        exit 1
    fi
}

# Create user-level directory layout
mkdir -p "$CAPSEM_DIR/bin" "$CAPSEM_DIR/assets" "$CAPSEM_DIR/run"

# Symlink system binaries into user dir
for bin in capsem capsem-service capsem-process capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray; do
    if [ -f "/usr/bin/$bin" ]; then
        ln -sf "/usr/bin/$bin" "$CAPSEM_DIR/bin/$bin"
    fi
done

seed_asset_manifests

# Fix ownership
chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$CAPSEM_DIR"

# Register systemd user unit and run setup (as the target user). These are
# release-critical: if either fails, dpkg must report failure instead of
# leaving a package that looks installed but cannot boot.
# XDG_RUNTIME_DIR is required for systemctl --user; su drops it.
TARGET_UID=$(id -u "$TARGET_USER")
XDG_DIR="/run/user/$TARGET_UID"
if command -v systemctl >/dev/null 2>&1; then
    su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR $CAPSEM_DIR/bin/capsem install"
fi
seed_asset_manifests
chown -R "$TARGET_USER:$(id -gn "$TARGET_USER")" "$CAPSEM_DIR/assets"
su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR $CAPSEM_DIR/bin/capsem setup --non-interactive --accept-detected"

exit 0
