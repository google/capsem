#!/bin/bash
# Replace ambient Ubuntu apt authorities with one signed immutable snapshot.
set -euo pipefail

snapshot_base="${1:?Ubuntu snapshot base is required}"
snapshot_id="${2:?Ubuntu snapshot ID is required}"

if [[ ! "$snapshot_base" =~ ^https://[^[:space:]]+$ ]]; then
    echo "ERROR: Ubuntu snapshot base must be an HTTPS URL" >&2
    exit 1
fi
if [[ ! "$snapshot_id" =~ ^[0-9]{8}T[0-9]{6}Z$ ]]; then
    echo "ERROR: invalid Ubuntu snapshot ID '$snapshot_id'" >&2
    exit 1
fi

native_arch="$(dpkg --print-architecture)"
case "$native_arch" in
    amd64 | arm64) ;;
    *)
        echo "ERROR: unsupported Ubuntu snapshot architecture '$native_arch'" >&2
        exit 1
        ;;
esac

snapshot_url="${snapshot_base%/}/${snapshot_id}"
rm -f /etc/apt/sources.list /etc/apt/sources.list.d/*
# The suite is the base image's own, not a name written here. A hardcoded
# `noble` installs 24.04 packages onto whatever base it is given, which is how
# lowering the release floor silently changed nothing -- see issue #181.
. /etc/os-release
suite="${UBUNTU_CODENAME:?base image declares no UBUNTU_CODENAME}"

cat > /etc/apt/sources.list.d/capsem-snapshot.sources <<EOF
Types: deb
URIs: $snapshot_url
Suites: $suite $suite-updates $suite-backports $suite-security
Components: main restricted universe multiverse
Architectures: $native_arch
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF
