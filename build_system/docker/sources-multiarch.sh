#!/bin/bash
# Configure apt sources for multiarch cross-compilation on Ubuntu 24.04.
#
# Both architectures come from one signed timestamped Ubuntu snapshot. The
# caller must select it explicitly; inheriting a mirror makes a cold rebuild
# resolve different -dev bytes under the same source revision.
set -euo pipefail

snapshot_base=${1:?Ubuntu snapshot base is required}
snapshot_id=${2:?Ubuntu snapshot ID is required}
if [[ ! "$snapshot_base" =~ ^https://[^[:space:]]+$ ]]; then
    echo "ERROR: Ubuntu snapshot base must be an HTTPS URL" >&2
    exit 1
fi
if [[ ! "$snapshot_id" =~ ^[0-9]{8}T[0-9]{6}Z$ ]]; then
    echo "ERROR: invalid Ubuntu snapshot ID '$snapshot_id'" >&2
    exit 1
fi

NATIVE_ARCH=$(dpkg --print-architecture)

if [ "$NATIVE_ARCH" = "arm64" ]; then
    FOREIGN_ARCH="amd64"
elif [ "$NATIVE_ARCH" = "amd64" ]; then
    FOREIGN_ARCH="arm64"
else
    echo "ERROR: unsupported native arch '$NATIVE_ARCH'"
    exit 1
fi

dpkg --add-architecture "$FOREIGN_ARCH"

# Write the foreign arch marker for later use in Dockerfile
echo "$FOREIGN_ARCH" > /tmp/foreign-arch

# Remove any existing sources to avoid conflicts
rm -f /etc/apt/sources.list /etc/apt/sources.list.d/*

# A partial update is not usable for cross-architecture package resolution.
# Retry transient mirror failures, then make any missing index fail the layer
# instead of silently reusing stale metadata.
cat > /etc/apt/apt.conf.d/80capsem-reliable-updates << 'EOF'
Acquire::Retries "5";
Acquire::http::Timeout "30";
Acquire::https::Timeout "30";
APT::Update::Error-Mode "any";
EOF

# Write one arch-scoped immutable source (DEB822 format).
snapshot_url="${snapshot_base%/}/${snapshot_id}"
cat > /etc/apt/sources.list.d/ubuntu.sources << EOF
Types: deb
URIs: $snapshot_url
Suites: noble noble-updates noble-backports noble-security
Components: main restricted universe multiverse
Architectures: $NATIVE_ARCH $FOREIGN_ARCH
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF

echo "Configured immutable multiarch snapshot: $snapshot_url ($NATIVE_ARCH $FOREIGN_ARCH)"
