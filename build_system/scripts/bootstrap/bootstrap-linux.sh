#!/bin/sh
# Linux host provisioning used by bootstrap.sh.
#
# This file only defines functions so its config parsing can be exercised by
# contract tests without installing packages or touching the host. bootstrap.sh
# sources it and calls bootstrap_linux on Linux.

CAPSEM_BOOTSTRAP_HELPER_DIR=${CAPSEM_BOOTSTRAP_HELPER_DIR:-build_system/scripts/bootstrap}
# shellcheck disable=SC1091
. "$CAPSEM_BOOTSTRAP_HELPER_DIR/bootstrap-linux-common.sh"
# shellcheck disable=SC1091
. "$CAPSEM_BOOTSTRAP_HELPER_DIR/bootstrap-linux-packages.sh"
# shellcheck disable=SC1091
. "$CAPSEM_BOOTSTRAP_HELPER_DIR/bootstrap-linux-host.sh"

bootstrap_linux() {
    CAPSEM_LINUX_PROJECT_ROOT=$1
    CAPSEM_LINUX_ASSUME_YES=$2
    CAPSEM_NET_DEV=${3:-/proc/net/dev}

    printf "\n== Provisioning Linux host ==\n"
    if command -v apt-get >/dev/null 2>&1; then
        capsem_linux_install_apt_packages "$CAPSEM_LINUX_PROJECT_ROOT" "$CAPSEM_LINUX_ASSUME_YES"
    elif command -v dnf >/dev/null 2>&1; then
        capsem_linux_install_dnf_packages "$CAPSEM_LINUX_PROJECT_ROOT" "$CAPSEM_LINUX_ASSUME_YES"
    else
        printf "  [FAIL] unsupported Linux package manager (expected apt-get or dnf)\n" >&2
        return 1
    fi

    capsem_linux_prepare_binfmt "$CAPSEM_NET_DEV"
    capsem_linux_prepare_bubblewrap "$CAPSEM_NET_DEV" "$CAPSEM_LINUX_PROJECT_ROOT"
    capsem_linux_install_node "$CAPSEM_LINUX_PROJECT_ROOT" "$CAPSEM_LINUX_ASSUME_YES"
    if capsem_linux_loopback_only "$CAPSEM_NET_DEV"; then
        capsem_linux_verify_docker
        capsem_linux_verify_vm_devices
    else
        capsem_linux_prepare_docker
        capsem_linux_prepare_vm_devices "$CAPSEM_LINUX_PROJECT_ROOT"
    fi
}
