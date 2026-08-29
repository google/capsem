#!/bin/sh
# Shared Linux bootstrap parsing and privilege helpers.

capsem_linux_node_major() {
    CAPSEM_NODE_CONFIG=$1
    awk -F= '
        /^[[:space:]]*node_major[[:space:]]*=/ {
            value = $2
            gsub(/[[:space:]]/, "", value)
            if (value !~ /^[0-9]+$/) {
                printf "invalid node_major in %s: %s\n", FILENAME, value > "/dev/stderr"
                invalid = 1
                next
            }
            if (count == 0) {
                required = value
            } else if (value != required) {
                printf "node_major must match across every architecture in %s\n", FILENAME > "/dev/stderr"
                invalid = 1
            }
            count++
        }
        END {
            if (invalid || count == 0) {
                exit 2
            }
            print required
        }
    ' "$CAPSEM_NODE_CONFIG"
}
capsem_linux_confirm() {
    CAPSEM_CONFIRM_LABEL=$1
    CAPSEM_CONFIRM_YES=$2
    if [ "$CAPSEM_CONFIRM_YES" = 1 ] || [ ! -t 0 ]; then
        return 0
    fi
    printf "  Install %s? [Y/n] " "$CAPSEM_CONFIRM_LABEL"
    read -r CAPSEM_CONFIRM_ANSWER
    case "$CAPSEM_CONFIRM_ANSWER" in
        n|N|no|NO) return 1 ;;
        *) return 0 ;;
    esac
}

capsem_linux_as_root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo "$@"
    else
        printf "  [FAIL] Linux host provisioning requires root or sudo: %s\n" "$*" >&2
        return 1
    fi
}

capsem_linux_bootstrap_user() {
    if [ "$(id -u)" -eq 0 ] \
        && [ -n "${SUDO_USER:-}" ] \
        && [ "${SUDO_USER:-root}" != root ]; then
        printf "%s\n" "$SUDO_USER"
    else
        id -un
    fi
}

capsem_linux_cross_arch() {
    CAPSEM_CROSS_MACHINE=${1:-$(uname -m)}
    case "$CAPSEM_CROSS_MACHINE" in
        x86_64|amd64) printf "aarch64\n" ;;
        aarch64|arm64) printf "x86_64\n" ;;
        *)
            printf "  [FAIL] unsupported Linux architecture for binfmt: %s\n" \
                "$CAPSEM_CROSS_MACHINE" >&2
            return 1 ;;
    esac
}

capsem_linux_apt_binfmt_package() {
    # Noble/Jammy ship the static bundle under this name. Resolute exposes the
    # registration package directly, so discover the archive's current name
    # rather than pinning bootstrap to one Ubuntu generation.
    for CAPSEM_APT_BINFMT_CANDIDATE in qemu-user-static qemu-user-binfmt; do
        if apt-cache show "$CAPSEM_APT_BINFMT_CANDIDATE" 2>/dev/null \
            | grep -q '^Package:'; then
            printf "%s\n" "$CAPSEM_APT_BINFMT_CANDIDATE"
            return 0
        fi
    done
    printf "  [FAIL] neither qemu-user-static nor qemu-user-binfmt is available from apt\n" >&2
    return 1
}

capsem_linux_dnf_binfmt_package() {
    case "$(capsem_linux_cross_arch)" in
        aarch64) printf "qemu-user-static-aarch64\n" ;;
        x86_64) printf "qemu-user-static-x86\n" ;;
    esac
}
