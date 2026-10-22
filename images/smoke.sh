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
    # These are inherited from the same base by terminal and graphical apps.
    # They must be available without starting a display, bus or background daemon.
    xpra --version
    for tool in dbus-run-session gnome-keyring-daemon secret-tool certutil; do
        command -v "$tool" >/dev/null
    done
    test -x /usr/local/bin/capsem-gui
    test -x /usr/lib/xorg/Xorg
    test ! -u /usr/lib/xorg/Xorg
    if find / -xdev \( -name "libcuda*" -o -name "libcublas*" -o -name "libhip*" -o -name "librocm*" \) 2>/dev/null | grep -q .; then
        echo "GPU runtime present" >&2
        exit 1
    fi
    if [ "$#" -gt 0 ]; then "$@"; fi
' smoke "$@"
echo "smoke ok: $image"
