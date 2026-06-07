#!/bin/bash
# Swap Tauri -dev libraries to the target architecture.
# Called at container runtime before cross-compilation.
#
# The image ships with native-arch -dev packages. If the Rust target
# is a different arch, we remove native -dev and install foreign -dev.
# If target matches native, this is a no-op.
#
# Usage: swap-dev-libs <target-arch>   (arm64 or amd64)
set -euo pipefail

TARGET_ARCH="${1:?usage: swap-dev-libs <arm64|amd64>}"
NATIVE_ARCH=$(dpkg --print-architecture)

if [ "$TARGET_ARCH" = "$NATIVE_ARCH" ]; then
    echo "Target matches native arch ($NATIVE_ARCH), no swap needed."
    exit 0
fi

DEV_PACKAGES=(
    libssl-dev
    libgtk-3-dev
    libwebkit2gtk-4.1-dev
    libayatana-appindicator3-dev
    librsvg2-dev
)

echo "Swapping -dev libs: $NATIVE_ARCH -> $TARGET_ARCH"

# Remove native-arch -dev packages
apt-get remove -y "${DEV_PACKAGES[@]}" >/dev/null 2>&1 || true

# Install foreign-arch -dev packages
FOREIGN_PKGS=()
for pkg in "${DEV_PACKAGES[@]}"; do
    FOREIGN_PKGS+=("${pkg}:${TARGET_ARCH}")
done

apt-get update -qq
apt-get install -y --no-install-recommends -o Dpkg::Options::="--force-overwrite" "${FOREIGN_PKGS[@]}"
rm -rf /var/lib/apt/lists/*

echo "Installed ${TARGET_ARCH} -dev libraries."
