#!/bin/sh
# Smoke an image the way CI does: its tools run as the unprivileged user, and
# no GPU runtime is present.
# Usage: images/smoke.sh <image> [agent command...]
set -eu

image="${1:?usage: images/smoke.sh <image> [agent command...]}"
shift

docker run --rm --network none "$image" sh -euc '
    test "$(id -u)" = 1000
    node --version
    python3 --version
    uv --version
    ollama --version 2>&1 | grep -F "version"
    if find / -xdev \( -name "libcuda*" -o -name "libcublas*" -o -name "libhip*" -o -name "librocm*" \) 2>/dev/null | grep -q .; then
        echo "GPU runtime present" >&2
        exit 1
    fi
    if [ "$#" -gt 0 ]; then "$@"; fi
' smoke "$@"
echo "smoke ok: $image"
