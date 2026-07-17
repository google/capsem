#!/usr/bin/env bash
# Ensure Docker has enough daemon-local disk for concurrent asset extraction.
set -euo pipefail

MINIMUM_GIB="${1:-16}"
if [[ ! "$MINIMUM_GIB" =~ ^[1-9][0-9]*$ ]]; then
    echo "ERROR: Docker minimum space must be a positive GiB integer (got: $MINIMUM_GIB)" >&2
    exit 2
fi

docker_free_kib() {
    docker run --rm debian:bookworm-slim sh -c \
        "df -Pk / | awk 'NR == 2 { print \$4 }'"
}

require_numeric_free_space() {
    local value="$1"
    if [[ ! "$value" =~ ^[0-9]+$ ]]; then
        echo "ERROR: could not measure Docker daemon free space (got: ${value:-<empty>})" >&2
        exit 1
    fi
}

MINIMUM_KIB=$((MINIMUM_GIB * 1024 * 1024))
FREE_KIB=$(docker_free_kib)
require_numeric_free_space "$FREE_KIB"

if (( FREE_KIB >= MINIMUM_KIB )); then
    echo "Docker already has $((FREE_KIB / 1024 / 1024)) GiB free (minimum: $MINIMUM_GIB GiB)."
    exit 0
fi

echo "Docker has only $((FREE_KIB / 1024 / 1024)) GiB free; pruning unused builder cache before parallel asset extraction."
docker builder prune -af >/dev/null

FREE_KIB=$(docker_free_kib)
require_numeric_free_space "$FREE_KIB"
if (( FREE_KIB < MINIMUM_KIB )); then
    echo "ERROR: parallel asset extraction requires at least $MINIMUM_GIB GiB free in Docker; only $((FREE_KIB / 1024 / 1024)) GiB remains after pruning unused builder cache." >&2
    docker system df >&2 || true
    exit 1
fi

echo "Docker reclaimed enough space: $((FREE_KIB / 1024 / 1024)) GiB free."
