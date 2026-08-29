#!/usr/bin/env bash
# Remove only Docker containers whose bind mounts belong to one build root.
set -euo pipefail

if [ "$#" -ne 1 ] || [ ! -d "$1" ]; then
    echo "usage: $0 <existing-mount-root>" >&2
    exit 2
fi

MOUNT_ROOT=$(cd "$1" && pwd -P)
while IFS= read -r container_id; do
    [ -n "$container_id" ] || continue
    while IFS= read -r source; do
        case "$source" in
            "$MOUNT_ROOT"|"$MOUNT_ROOT"/*)
                echo "Removing interrupted asset container $container_id (mount: $source)"
                docker rm -f "$container_id" >/dev/null
                break
                ;;
        esac
    done < <(docker inspect --format '{{range .Mounts}}{{println .Source}}{{end}}' "$container_id" 2>/dev/null || true)
done < <(docker ps -aq)
