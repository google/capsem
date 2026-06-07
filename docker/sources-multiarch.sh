#!/bin/bash
# Configure apt sources for multiarch cross-compilation on Ubuntu 24.04.
#
# Ubuntu arm64 images use ports.ubuntu.com (which only has arm64/armhf/etc).
# Ubuntu amd64 images use archive.ubuntu.com (which only has amd64/i386).
# Security updates follow the same split: arm64 from ports, amd64 from
# security.ubuntu.com. For multiarch we need both repos, scoped by arch.
set -euo pipefail

NATIVE_ARCH=$(dpkg --print-architecture)

if [ "$NATIVE_ARCH" = "arm64" ]; then
    FOREIGN_ARCH="amd64"
    NATIVE_MIRROR="http://ports.ubuntu.com/ubuntu-ports"
    NATIVE_SECURITY="http://ports.ubuntu.com/ubuntu-ports"
    FOREIGN_MIRROR="http://archive.ubuntu.com/ubuntu"
    FOREIGN_SECURITY="http://security.ubuntu.com/ubuntu"
elif [ "$NATIVE_ARCH" = "amd64" ]; then
    FOREIGN_ARCH="arm64"
    NATIVE_MIRROR="http://archive.ubuntu.com/ubuntu"
    NATIVE_SECURITY="http://security.ubuntu.com/ubuntu"
    FOREIGN_MIRROR="http://ports.ubuntu.com/ubuntu-ports"
    FOREIGN_SECURITY="http://ports.ubuntu.com/ubuntu-ports"
else
    echo "ERROR: unsupported native arch '$NATIVE_ARCH'"
    exit 1
fi

dpkg --add-architecture "$FOREIGN_ARCH"

# Write the foreign arch marker for later use in Dockerfile
echo "$FOREIGN_ARCH" > /tmp/foreign-arch

# Remove any existing sources to avoid conflicts
rm -f /etc/apt/sources.list /etc/apt/sources.list.d/*

# Write arch-scoped multiarch sources (DEB822 format)
cat > /etc/apt/sources.list.d/ubuntu.sources << EOF
Types: deb
URIs: $NATIVE_MIRROR
Suites: noble noble-updates noble-backports
Components: main restricted universe multiverse
Architectures: $NATIVE_ARCH
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: $NATIVE_SECURITY
Suites: noble-security
Components: main restricted universe multiverse
Architectures: $NATIVE_ARCH
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: $FOREIGN_MIRROR
Suites: noble noble-updates noble-backports
Components: main restricted universe multiverse
Architectures: $FOREIGN_ARCH
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: $FOREIGN_SECURITY
Suites: noble-security
Components: main restricted universe multiverse
Architectures: $FOREIGN_ARCH
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF

echo "Configured multiarch: native=$NATIVE_ARCH ($NATIVE_MIRROR), foreign=$FOREIGN_ARCH ($FOREIGN_MIRROR)"
