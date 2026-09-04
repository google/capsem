#!/bin/sh
# Linux bootstrap binfmt, sandbox, Docker, and VM-device helpers.

capsem_linux_network_interfaces() {
    CAPSEM_NET_DEV=${1:-/proc/net/dev}
    [ -r "$CAPSEM_NET_DEV" ] || return 1
    awk -F: '
        NR > 2 {
            interface = $1
            gsub(/[[:space:]]/, "", interface)
            if (interface != "") print interface
        }
    ' "$CAPSEM_NET_DEV" | sort
}

capsem_linux_loopback_only() {
    [ "$(capsem_linux_network_interfaces "${1:-/proc/net/dev}")" = lo ]
}

capsem_linux_binfmt_entry() {
    CAPSEM_BINFMT_ROOT=${1:-/proc/sys/fs/binfmt_misc}
    CAPSEM_BINFMT_ARCH=${2:-$(capsem_linux_cross_arch)}
    for CAPSEM_BINFMT_CANDIDATE in \
        "$CAPSEM_BINFMT_ROOT/qemu-$CAPSEM_BINFMT_ARCH" \
        "$CAPSEM_BINFMT_ROOT/qemu-$CAPSEM_BINFMT_ARCH-static"; do
        if [ -f "$CAPSEM_BINFMT_CANDIDATE" ]; then
            printf "%s\n" "$CAPSEM_BINFMT_CANDIDATE"
            return 0
        fi
    done
    return 1
}

capsem_linux_verify_binfmt() {
    CAPSEM_BINFMT_ROOT=${1:-/proc/sys/fs/binfmt_misc}
    CAPSEM_BINFMT_ARCH=${2:-$(capsem_linux_cross_arch)}
    CAPSEM_BINFMT_ENTRY=$(capsem_linux_binfmt_entry \
        "$CAPSEM_BINFMT_ROOT" "$CAPSEM_BINFMT_ARCH" || true)
    if [ -z "$CAPSEM_BINFMT_ENTRY" ]; then
        printf "  [FAIL] no enabled qemu-%s binfmt registration\n" \
            "$CAPSEM_BINFMT_ARCH" >&2
        return 1
    fi
    if ! grep -qx 'enabled' "$CAPSEM_BINFMT_ENTRY"; then
        printf "  [FAIL] qemu-%s binfmt registration is disabled\n" \
            "$CAPSEM_BINFMT_ARCH" >&2
        return 1
    fi
    CAPSEM_BINFMT_FLAGS=$(awk -F': ' '$1 == "flags" { print $2; exit }' \
        "$CAPSEM_BINFMT_ENTRY")
    case "$CAPSEM_BINFMT_FLAGS" in
        *F*) ;;
        *)
            printf "  [FAIL] qemu-%s binfmt lacks the fix-binary flag Docker requires\n" \
                "$CAPSEM_BINFMT_ARCH" >&2
            return 1 ;;
    esac
    CAPSEM_BINFMT_INTERPRETER=$(awk '$1 == "interpreter" { print $2; exit }' \
        "$CAPSEM_BINFMT_ENTRY")
    if [ -z "$CAPSEM_BINFMT_INTERPRETER" ] \
        || [ ! -x "$CAPSEM_BINFMT_INTERPRETER" ]; then
        printf "  [FAIL] qemu-%s binfmt interpreter is missing or not executable: %s\n" \
            "$CAPSEM_BINFMT_ARCH" "${CAPSEM_BINFMT_INTERPRETER:-none}" >&2
        return 1
    fi
    printf "  [ok]   qemu-%s binfmt (Docker %s container execution)\n" \
        "$CAPSEM_BINFMT_ARCH" "$CAPSEM_BINFMT_ARCH"
}

capsem_linux_prepare_binfmt() {
    CAPSEM_BINFMT_NET_DEV=${1:-/proc/net/dev}
    if capsem_linux_verify_binfmt >/dev/null 2>&1; then
        capsem_linux_verify_binfmt
        return 0
    fi
    if capsem_linux_loopback_only "$CAPSEM_BINFMT_NET_DEV"; then
        printf "         Run ./bootstrap.sh outside the gate to register distro QEMU.\n" >&2
        return 1
    fi

    # The distro owns the magic/mask and interpreter path. Refresh its
    # registration service instead of copying those architecture constants
    # into Capsem or using a privileged mutable helper image.
    if command -v systemctl >/dev/null 2>&1; then
        capsem_linux_as_root systemctl restart systemd-binfmt.service || true
    fi
    if command -v update-binfmts >/dev/null 2>&1; then
        capsem_linux_as_root update-binfmts --enable || true
    fi
    capsem_linux_verify_binfmt
}

capsem_linux_prepare_bubblewrap() {
    CAPSEM_NET_DEV=${1:-/proc/net/dev}
    CAPSEM_BUBBLEWRAP_PROJECT_ROOT=${2:-}
    if capsem_linux_loopback_only "$CAPSEM_NET_DEV"; then
        if ! { : > /dev/null; } 2>/dev/null; then
            printf "  [FAIL] active Linux gate namespace cannot use /dev/null\n" >&2
            return 1
        fi
        printf "  [ok]   Bubblewrap network namespace already active (loopback only)\n"
        return 0
    fi
    if [ -n "$CAPSEM_BUBBLEWRAP_PROJECT_ROOT" ]; then
        python3 "$CAPSEM_BUBBLEWRAP_PROJECT_ROOT/build_system/scripts/bootstrap/prepare-linux-sandbox.py" --repair-hosted-runner
        return
    fi
    if ! bwrap --unshare-net --die-with-parent --new-session \
        --bind / / --dev-bind /dev /dev -- sh -c ': > /dev/null' \
        >/dev/null 2>&1; then
        printf "  [FAIL] Bubblewrap cannot create a usable Linux gate namespace\n" >&2
        printf "         Check that unprivileged user namespaces are enabled.\n" >&2
        return 1
    fi
    printf "  [ok]   Bubblewrap network namespace and device mount\n"
}

capsem_linux_prepare_docker() {
    CAPSEM_BOOTSTRAP_USER=$(capsem_linux_bootstrap_user)
    if ! id "$CAPSEM_BOOTSTRAP_USER" >/dev/null 2>&1; then
        printf "  [FAIL] bootstrap user does not exist: %s\n" "$CAPSEM_BOOTSTRAP_USER" >&2
        return 1
    fi

    if ! docker info >/dev/null 2>&1; then
        if command -v systemctl >/dev/null 2>&1 \
            && capsem_linux_as_root systemctl enable --now docker; then
            :
        elif command -v service >/dev/null 2>&1 \
            && capsem_linux_as_root service docker start; then
            :
        else
            printf "  [FAIL] Docker daemon is unavailable and could not be started\n" >&2
            return 1
        fi
    fi

    # `systemctl start` returning only means activation was accepted. Wait for
    # the socket before repairing current-session access so a fresh host does
    # not race the daemon it just enabled.
    CAPSEM_DOCKER_WAIT=0
    while [ ! -S /var/run/docker.sock ] && [ "$CAPSEM_DOCKER_WAIT" -lt 30 ]; do
        sleep 1
        CAPSEM_DOCKER_WAIT=$((CAPSEM_DOCKER_WAIT + 1))
    done

    if [ "$(id -u "$CAPSEM_BOOTSTRAP_USER")" -ne 0 ] \
        && getent group docker >/dev/null 2>&1; then
        capsem_linux_as_root usermod -aG docker "$CAPSEM_BOOTSTRAP_USER"
    fi
    if ! docker info >/dev/null 2>&1 && [ -S /var/run/docker.sock ]; then
        capsem_linux_as_root setfacl -m "u:$CAPSEM_BOOTSTRAP_USER:rw" /var/run/docker.sock
    fi
    CAPSEM_DOCKER_ACCESS_WAIT=0
    while ! docker info >/dev/null 2>&1 \
        && [ "$CAPSEM_DOCKER_ACCESS_WAIT" -lt 30 ]; do
        sleep 1
        CAPSEM_DOCKER_ACCESS_WAIT=$((CAPSEM_DOCKER_ACCESS_WAIT + 1))
    done
    if ! docker info >/dev/null 2>&1; then
        printf "  [FAIL] Docker daemon is not accessible after provisioning\n" >&2
        return 1
    fi
    if ! docker buildx version >/dev/null 2>&1; then
        printf "  [FAIL] docker buildx is not available after provisioning\n" >&2
        return 1
    fi
    printf "  [ok]   Docker daemon + Buildx (current-session access)\n"
}

capsem_linux_verify_docker() {
    if ! docker info >/dev/null 2>&1; then
        printf "  [FAIL] Docker daemon is not accessible inside the Linux gate\n" >&2
        printf "         Run ./bootstrap.sh outside the gate to provision host access.\n" >&2
        return 1
    fi
    if ! docker buildx version >/dev/null 2>&1; then
        printf "  [FAIL] docker buildx is not available inside the Linux gate\n" >&2
        printf "         Run ./bootstrap.sh outside the gate to provision it.\n" >&2
        return 1
    fi
    printf "  [ok]   Docker daemon + Buildx (existing gate access)\n"
}

capsem_linux_verify_vm_devices() {
    if [ ! -e /dev/kvm ]; then
        printf "  [WARN] /dev/kvm not found; VM boot tests require KVM\n"
    elif ! { [ -r /dev/kvm ] && [ -w /dev/kvm ]; }; then
        printf "  [FAIL] /dev/kvm is not readable and writable\n" >&2
        printf "         Run ./bootstrap.sh outside the gate to provision host access.\n" >&2
        return 1
    else
        printf "  [ok]   /dev/kvm (durable current-session access)\n"
    fi

    if [ ! -e /dev/vhost-vsock ]; then
        printf "  [WARN] /dev/vhost-vsock not found; VM guest communication requires vhost_vsock\n"
    elif ! { [ -r /dev/vhost-vsock ] && [ -w /dev/vhost-vsock ]; }; then
        printf "  [FAIL] /dev/vhost-vsock is not readable and writable\n" >&2
        printf "         Run ./bootstrap.sh outside the gate to provision host access.\n" >&2
        return 1
    else
        printf "  [ok]   /dev/vhost-vsock (durable current-session access)\n"
    fi
}

capsem_linux_prepare_vm_devices() {
    CAPSEM_VM_DEVICE_PROJECT_ROOT=${1:?capsem_linux_prepare_vm_devices <project-root>}
    CAPSEM_BOOTSTRAP_USER=$(capsem_linux_bootstrap_user)
    CAPSEM_VM_DEVICE_HELPER="$CAPSEM_VM_DEVICE_PROJECT_ROOT/build_system/packaging/shared/install-vm-device-access"
    if [ ! -f "$CAPSEM_VM_DEVICE_HELPER" ]; then
        printf "  [FAIL] VM-device helper is missing: %s\n" "$CAPSEM_VM_DEVICE_HELPER" >&2
        return 1
    fi
    capsem_linux_as_root bash "$CAPSEM_VM_DEVICE_HELPER" "$CAPSEM_BOOTSTRAP_USER" \
        "$CAPSEM_VM_DEVICE_PROJECT_ROOT/build_system/packaging/linux/99-capsem-vm-devices.rules"

    capsem_linux_verify_vm_devices
}
