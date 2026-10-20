#!/bin/bash
# Install the systemd harness and package runtime dependencies from one
# immutable Ubuntu snapshot. This runs only in the network-open helper stage.
set -euo pipefail

: "${APT_UPDATE_TIMEOUT_SECONDS:?apt update timeout is required}"
: "${APT_INSTALL_TIMEOUT_SECONDS:?apt install timeout is required}"
: "${APT_REQUEST_TIMEOUT_SECONDS:?apt request timeout is required}"
: "${APT_RETRIES:?apt retry count is required}"
# shellcheck source=build_system/docker/apt-with-retries.sh
source "$(dirname "${BASH_SOURCE[0]}")/apt-with-retries.sh"

snapshot_base="${1:?Ubuntu snapshot base is required}"
snapshot_id="${2:?Ubuntu snapshot ID is required}"
shift 2

if [[ ! "$snapshot_base" =~ ^https://[^[:space:]]+$ ]]; then
    echo "ERROR: Ubuntu snapshot base must be an HTTPS URL" >&2
    exit 1
fi
if [[ ! "$snapshot_id" =~ ^[0-9]{8}T[0-9]{6}Z$ ]]; then
    echo "ERROR: invalid Ubuntu snapshot ID '$snapshot_id'" >&2
    exit 1
fi
if [ "$#" -eq 0 ]; then
    echo "ERROR: at least one install package is required" >&2
    exit 1
fi
for package in "$@"; do
    if [[ ! "$package" =~ ^[a-z0-9][a-z0-9+.-]*$ ]]; then
        echo "ERROR: invalid Ubuntu package name '$package'" >&2
        exit 1
    fi
done

native_arch="$(dpkg --print-architecture)"
case "$native_arch" in
    amd64 | arm64) ;;
    *)
        echo "ERROR: unsupported install helper architecture '$native_arch'" >&2
        exit 1
        ;;
esac

configure-apt-snapshot "$snapshot_base" "$snapshot_id"
capsem_apt_with_retry "$APT_UPDATE_TIMEOUT_SECONDS" apt-get update -qq \
    -o Acquire::https::Timeout="$APT_REQUEST_TIMEOUT_SECONDS" \
    -o Acquire::Retries="$APT_RETRIES"
install_attempt_timeout=$(capsem_apt_install_attempt_timeout)
capsem_apt_with_retry "$install_attempt_timeout" apt-get install \
    -y --reinstall --allow-downgrades --no-install-recommends \
    -o Acquire::https::Timeout="$APT_REQUEST_TIMEOUT_SECONDS" \
    -o Acquire::Retries="$APT_RETRIES" "$@"
