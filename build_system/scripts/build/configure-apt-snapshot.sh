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
cat > /etc/apt/sources.list.d/capsem-snapshot.sources <<EOF
Types: deb
URIs: $snapshot_url
Suites: noble noble-updates noble-backports noble-security
Components: main restricted universe multiverse
Architectures: $native_arch
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF

# The snapshot is the authority even over what the image already carries. A
# hosted runner image moves past the snapshot between rebuilds (mesa 04.4
# preinstalled against the snapshot's 04.2), and apt then refuses any install
# that needs the snapshot's version of a newer installed library. Above 1000
# lets the transaction downgrade those to the snapshot cohort.
snapshot_host="${snapshot_base#https://}"
snapshot_host="${snapshot_host%%/*}"
rm -f /etc/apt/preferences /etc/apt/preferences.d/*
cat > /etc/apt/preferences.d/capsem-snapshot.pref <<EOF
Package: *
Pin: origin "$snapshot_host"
Pin-Priority: 1001
EOF
