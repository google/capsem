#!/bin/bash
# Create or clear the root-owned pre-activation package manifest handoff.
set -euo pipefail

REQUEST_DIR="/var/run/capsem"
REQUEST="$REQUEST_DIR/install-manifest"
PAYLOAD="$REQUEST.json"
action="${1:?usage: install-manifest-request.sh write <manifest-path> [logical-source] | clear}"

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
        logical_source="${3:-}"
        if [ -n "$logical_source" ]; then
            case "$logical_source" in
                file:///*|http://*|https://*) ;;
                *)
                    echo "capsem: install manifest logical source must be a URL: $logical_source" >&2
                    exit 2
                    ;;
            esac
            if [[ "$logical_source" == *"%"* ]] \
                || [[ "$logical_source" == *[!A-Za-z0-9/:._+-]* ]]; then
                echo "capsem: install manifest logical source contains unsafe characters" >&2
                exit 2
            fi
        fi
        source_file=$(mktemp "${TMPDIR:-/tmp}/capsem-install-manifest.XXXXXX")
        trap 'rm -f "$source_file"' EXIT
        if [ -n "$logical_source" ]; then
            printf '%s\nfile://%s\n' "$logical_source" "$PAYLOAD" > "$source_file"
        else
            printf 'file://%s\n' "$manifest" > "$source_file"
        fi
        chmod 0600 "$source_file"
        group=root
        if [ "$(uname -s)" = "Darwin" ]; then
            group=wheel
        fi
        run_root install -d -o root -g "$group" -m 0700 "$REQUEST_DIR"
        if [ -n "$logical_source" ]; then
            run_root install -o root -g "$group" -m 0600 "$manifest" "$PAYLOAD"
        else
            run_root rm -f "$PAYLOAD"
        fi
        run_root install -o root -g "$group" -m 0600 "$source_file" "$REQUEST"
        ;;
    clear)
        run_root rm -f "$REQUEST" "$PAYLOAD"
        ;;
    *)
        echo "usage: install-manifest-request.sh write <manifest-path> [logical-source] | clear" >&2
        exit 2
        ;;
esac
