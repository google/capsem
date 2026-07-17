#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 5 ]; then
    echo "usage: $0 <host-os> <host-arch> <target-arch> <kvm-ready:0|1> <required:0|1>" >&2
    exit 2
fi

HOST_OS=$1
HOST_ARCH=$2
TARGET_ARCH=$3
KVM_READY=$4
REQUIRED=$5

case "$KVM_READY:$REQUIRED" in
    0:0|0:1|1:0|1:1) ;;
    *)
        echo "ERROR: kvm-ready and required must be 0 or 1" >&2
        exit 2
        ;;
esac

# A two-architecture build always emits one non-host package. Requiring a
# native KVM proof must not reject that structurally validated cross artifact;
# the matching native qualification runner owns its functional proof.
if [ "$HOST_OS" != "Linux" ] || [ "$TARGET_ARCH" != "$HOST_ARCH" ]; then
    echo skip
    exit 0
fi

if [ "$KVM_READY" = "1" ]; then
    echo prove
    exit 0
fi

if [ "$REQUIRED" = "1" ]; then
    echo "ERROR: native Linux package proof requires KVM and vhost-vsock" >&2
    echo "       host=$HOST_OS/$HOST_ARCH target=$TARGET_ARCH" >&2
    exit 1
fi

echo skip
