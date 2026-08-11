#!/bin/bash
# Swap Tauri -dev libraries to the target architecture.
# Called at container runtime before cross-compilation.
#
# The image ships with native-arch -dev packages. If the Rust target
# is a different arch, we remove native -dev and install foreign -dev.
# If target matches native, the same packages are reinstalled from the selected
# snapshot so mutable host-builder bytes cannot enter the package.
#
# Usage: swap-dev-libs <target-arch> <snapshot-base> <snapshot-id>
set -euo pipefail

TARGET_ARCH="${1:?target architecture is required}"
APT_SNAPSHOT_BASE="${2:?Ubuntu snapshot base is required}"
APT_SNAPSHOT_ID="${3:?Ubuntu snapshot ID is required}"
if [[ ! "$APT_SNAPSHOT_BASE" =~ ^https://[^[:space:]]+$ ]]; then
    echo "ERROR: Ubuntu snapshot base must be an HTTPS URL" >&2
    exit 1
fi
if [[ ! "$APT_SNAPSHOT_ID" =~ ^[0-9]{8}T[0-9]{6}Z$ ]]; then
    echo "ERROR: invalid Ubuntu snapshot ID '$APT_SNAPSHOT_ID'" >&2
    exit 1
fi
SNAPSHOT_URL="${APT_SNAPSHOT_BASE%/}/${APT_SNAPSHOT_ID}"
NATIVE_ARCH=$(dpkg --print-architecture)

case "$NATIVE_ARCH" in
    arm64) FOREIGN_ARCH=amd64 ;;
    amd64) FOREIGN_ARCH=arm64 ;;
    *)
        echo "ERROR: unsupported native architecture '$NATIVE_ARCH'" >&2
        exit 1
        ;;
esac
if [ "$TARGET_ARCH" != "$NATIVE_ARCH" ] && [ "$TARGET_ARCH" != "$FOREIGN_ARCH" ]; then
    echo "ERROR: target '$TARGET_ARCH' is neither native nor configured foreign architecture" >&2
    exit 1
fi

dpkg --add-architecture "$FOREIGN_ARCH"
# Replace every mutable archive inherited from the host builder before any
# helper-layer apt operation. The official direct timestamped endpoint is
# immutable and serves both configured architectures and every Noble pocket.
rm -f /etc/apt/sources.list /etc/apt/sources.list.d/*
cat > /etc/apt/sources.list.d/capsem-snapshot.sources << EOF
Types: deb
URIs: $SNAPSHOT_URL
Suites: noble noble-updates noble-backports noble-security
Components: main restricted universe multiverse
Architectures: $NATIVE_ARCH $FOREIGN_ARCH
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF

DEV_PACKAGES=(
    libssl-dev
    libgtk-3-dev
    libwebkit2gtk-4.1-dev
    libayatana-appindicator3-dev
    libxdo-dev
)

# Prove every architecture-scoped index is fresh before mutating the installed
# toolchain. The image-level apt policy retries transient downloads and makes a
# partial update fatal.
apt-get update -qq

if [ "$TARGET_ARCH" = "$NATIVE_ARCH" ]; then
    NATIVE_PKGS=()
    for pkg in "${DEV_PACKAGES[@]}"; do
        NATIVE_PKGS+=("${pkg}:${NATIVE_ARCH}")
    done
    echo "Reinstalling $NATIVE_ARCH dev libraries via $SNAPSHOT_URL"
    apt-get install -y --reinstall --allow-downgrades --no-install-recommends \
        -o Dpkg::Options::="--force-overwrite" "${NATIVE_PKGS[@]}"
    rm -rf /var/lib/apt/lists/*
    echo "Reinstalled $NATIVE_ARCH dev libraries from the selected snapshot."
    exit 0
fi

echo "Swapping -dev libs from $NATIVE_ARCH to $TARGET_ARCH via $SNAPSHOT_URL"

# Remove native-arch -dev packages only after the foreign indexes are usable.
apt-get remove -y "${DEV_PACKAGES[@]}"

# Install foreign-arch -dev packages
FOREIGN_PKGS=()
for pkg in "${DEV_PACKAGES[@]}"; do
    FOREIGN_PKGS+=("${pkg}:${TARGET_ARCH}")
done

apt-get install -y --no-install-recommends -o Dpkg::Options::="--force-overwrite" "${FOREIGN_PKGS[@]}"
rm -rf /var/lib/apt/lists/*

echo "Installed ${TARGET_ARCH} -dev libraries."
