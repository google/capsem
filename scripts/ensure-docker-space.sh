#!/usr/bin/env bash
# Ensure Docker has enough daemon-local disk for build/test rails.
set -euo pipefail

MINIMUM_GIB="${1:-16}"
if [[ ! "$MINIMUM_GIB" =~ ^[1-9][0-9]*$ ]]; then
    echo "ERROR: Docker minimum space must be a positive GiB integer (got: $MINIMUM_GIB)" >&2
    exit 2
fi
CACHE_KEEP_GIB="${CAPSEM_DOCKER_CACHE_KEEP_GB:-8}"
if [[ ! "$CACHE_KEEP_GIB" =~ ^[1-9][0-9]*$ ]]; then
    echo "ERROR: Docker cache floor must be a positive GiB integer (got: $CACHE_KEEP_GIB)" >&2
    exit 2
fi
LINKED_KEEP_GIB="${CAPSEM_DOCKER_LINKED_KEEP_GB:-4}"
if [[ ! "$LINKED_KEEP_GIB" =~ ^[1-9][0-9]*$ ]]; then
    echo "ERROR: Docker linked-artifact floor must be a positive GiB integer (got: $LINKED_KEEP_GIB)" >&2
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

echo "Docker has only $((FREE_KIB / 1024 / 1024)) GiB free; pruning unused builder cache while retaining the hottest $CACHE_KEEP_GIB GiB."
docker builder prune -f --keep-storage "${CACHE_KEEP_GIB}GB" >/dev/null

FREE_KIB=$(docker_free_kib)
require_numeric_free_space "$FREE_KIB"
if (( FREE_KIB >= MINIMUM_KIB )); then
    echo "Docker reclaimed enough space: $((FREE_KIB / 1024 / 1024)) GiB free."
    exit 0
fi

# Cargo target volumes retain hash-suffixed artifacts forever. Preserve compiled
# dependencies for fast focused retries, but discard incremental compiler state
# from inactive Capsem target volumes when BuildKit pruning was insufficient.
# Never touch a mounted volume: another rail or worktree may be using it.
while IFS= read -r volume; do
    [[ "$volume" == capsem-*-target* ]] || continue
    if [ -n "$(docker ps -q --filter "volume=$volume")" ]; then
        echo "preserving active Cargo target volume: $volume"
        continue
    fi
    echo "trimming inactive Cargo incremental cache: $volume"
    docker run --rm \
        -v "$volume:/cargo-target" \
        alpine:3.20 \
        sh -c 'find /cargo-target -type d -name incremental -prune -exec rm -rf {} +'
    FREE_KIB=$(docker_free_kib)
    require_numeric_free_space "$FREE_KIB"
    if (( FREE_KIB >= MINIMUM_KIB )); then
        echo "Docker reclaimed enough space: $((FREE_KIB / 1024 / 1024)) GiB free."
        exit 0
    fi
done < <(docker volume ls --format '{{.Name}}')

# Linked test/application executables have no extension under Cargo deps/ and
# are cheap to recreate, but they can silently accumulate across hash changes
# (6.4 GiB observed in capsem-install-target). If cache and incremental pruning
# were insufficient, retain only the newest cohort in inactive target volumes.
# Dependency libraries (.rlib/.rmeta/etc.) are deliberately untouched.
while IFS= read -r volume; do
    [[ "$volume" == capsem-*-target* ]] || continue
    if [ -n "$(docker ps -q --filter "volume=$volume")" ]; then
        echo "preserving active Cargo target volume: $volume"
        continue
    fi
    echo "trimming inactive Cargo linked artifacts: $volume (keeping newest $LINKED_KEEP_GIB GiB)"
    docker run --rm \
        -e LINKED_KEEP_GIB="$LINKED_KEEP_GIB" \
        -v "$volume:/cargo-target" \
        alpine:3.20 \
        sh -ceu '
            keep_bytes=$((LINKED_KEEP_GIB * 1024 * 1024 * 1024))
            for deps in \
                /cargo-target/debug/deps \
                /cargo-target/release/deps \
                /cargo-target/llvm-cov-target/debug/deps
            do
                [ -d "$deps" ] || continue
                listing=$(mktemp)
                find "$deps" -maxdepth 1 -type f ! -name "*.*" \
                    -exec stat -c "%Y %s %n" {} + | sort -n > "$listing"
                total=$(awk "{ total += \$2 } END { print total + 0 }" "$listing")
                while read -r _mtime size path; do
                    [ "$total" -gt "$keep_bytes" ] || break
                    rm -f "$path"
                    total=$((total - size))
                done < "$listing"
                rm -f "$listing"
            done
        '
    FREE_KIB=$(docker_free_kib)
    require_numeric_free_space "$FREE_KIB"
    if (( FREE_KIB >= MINIMUM_KIB )); then
        echo "Docker reclaimed enough space: $((FREE_KIB / 1024 / 1024)) GiB free."
        exit 0
    fi
done < <(docker volume ls --format '{{.Name}}')

echo "ERROR: Docker capacity gate requires at least $MINIMUM_GIB GiB free; only $((FREE_KIB / 1024 / 1024)) GiB remains after pruning BuildKit and inactive incremental/linked artifacts." >&2
docker system df -v >&2 || true
exit 1
