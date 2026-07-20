#!/bin/bash
# Create or clear PackageKit's deterministic handoff for Capsem's target user.
set -euo pipefail

CAPSEM_INSTALL_USER_REQUEST_DIR="/var/run/capsem"
CAPSEM_INSTALL_USER_REQUEST="$CAPSEM_INSTALL_USER_REQUEST_DIR/install-user"
action="${1:?usage: macos-install-user-request.sh write [user] | clear}"

case "$action" in
    write)
        install_user="${2:-${SUDO_USER:-$(id -un)}}"
        case "$install_user" in
            ""|root|*[!A-Za-z0-9._-]*)
                echo "capsem: refusing invalid macOS install user: ${install_user:-<empty>}" >&2
                exit 1
                ;;
        esac
        install_uid=$(id -u "$install_user" 2>/dev/null) || {
            echo "capsem: macOS install user does not exist: $install_user" >&2
            exit 1
        }
        [ "$install_uid" -ne 0 ] || {
            echo "capsem: macOS install user must be non-root" >&2
            exit 1
        }
        source_file=$(mktemp "${TMPDIR:-/tmp}/capsem-install-user.XXXXXX")
        trap 'rm -f "$source_file"' EXIT
        printf '%s\n' "$install_user" > "$source_file"
        chmod 0600 "$source_file"
        if [ "$(id -u)" -eq 0 ]; then
            /usr/bin/install -d -o root -g wheel -m 0700 "$CAPSEM_INSTALL_USER_REQUEST_DIR"
            /usr/bin/install -o root -g wheel -m 0600 "$source_file" "$CAPSEM_INSTALL_USER_REQUEST"
        else
            sudo /usr/bin/install -d -o root -g wheel -m 0700 "$CAPSEM_INSTALL_USER_REQUEST_DIR"
            sudo /usr/bin/install -o root -g wheel -m 0600 "$source_file" "$CAPSEM_INSTALL_USER_REQUEST"
        fi
        ;;
    clear)
        if [ "$(id -u)" -eq 0 ]; then
            rm -f "$CAPSEM_INSTALL_USER_REQUEST"
        else
            sudo rm -f "$CAPSEM_INSTALL_USER_REQUEST"
        fi
        ;;
    *)
        echo "usage: macos-install-user-request.sh write [user] | clear" >&2
        exit 2
        ;;
esac
