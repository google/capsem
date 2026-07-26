#!/bin/bash
# Create or clear the root-owned pre-activation package manifest handoff.
set -euo pipefail

REQUEST_DIR="/var/run/capsem"
REQUEST="$REQUEST_DIR/install-manifest"
action="${1:?usage: install-manifest-request.sh write <manifest-path> | clear}"

run_root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    else
        sudo "$@"
    fi
}

case "$action" in
    write)
        manifest="${2:?write requires an absolute serialized manifest path}"
        case "$manifest" in
            /*) ;;
            *)
                echo "capsem: install manifest path must be absolute: $manifest" >&2
                exit 2
                ;;
        esac
        if [ ! -f "$manifest" ] || [ -L "$manifest" ]; then
            echo "capsem: install manifest must be a regular file: $manifest" >&2
            exit 1
        fi
        source_file=$(mktemp "${TMPDIR:-/tmp}/capsem-install-manifest.XXXXXX")
        trap 'rm -f "$source_file"' EXIT
        printf 'file://%s\n' "$manifest" > "$source_file"
        chmod 0600 "$source_file"
        group=root
        if [ "$(uname -s)" = "Darwin" ]; then
            group=wheel
        fi
        run_root install -d -o root -g "$group" -m 0700 "$REQUEST_DIR"
        run_root install -o root -g "$group" -m 0600 "$source_file" "$REQUEST"
        ;;
    clear)
        run_root rm -f "$REQUEST"
        ;;
    *)
        echo "usage: install-manifest-request.sh write <manifest-path> | clear" >&2
        exit 2
        ;;
esac
