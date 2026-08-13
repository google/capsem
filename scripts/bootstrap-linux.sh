#!/bin/sh
# Linux host provisioning used by bootstrap.sh.
#
# This file only defines functions so its config parsing can be exercised by
# contract tests without installing packages or touching the host. bootstrap.sh
# sources it and calls bootstrap_linux on Linux.

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

capsem_linux_install_apt_packages() {
    CAPSEM_APT_PROJECT_ROOT=$1
    CAPSEM_APT_ASSUME_YES=$2
    CAPSEM_APT_BINFMT_PACKAGE=$(capsem_linux_apt_binfmt_package)
    CAPSEM_APT_WORKSPACE_PACKAGES=$(python3 \
        "$CAPSEM_APT_PROJECT_ROOT/scripts/provision-linux-workspace.py" --packages apt)
    CAPSEM_APT_DOCKER_PACKAGES=$("$CAPSEM_APT_PROJECT_ROOT/scripts/select-docker-packages.sh")
    CAPSEM_APT_BASE_PACKAGES="
        acl
        ca-certificates
        cpio
        python3
        python3-venv
        sqlite3
        util-linux
        xvfb
        xz-utils
        zstd
        $CAPSEM_APT_WORKSPACE_PACKAGES
        $CAPSEM_APT_BINFMT_PACKAGE
        $CAPSEM_APT_DOCKER_PACKAGES
    "

    CAPSEM_APT_NEEDS_INSTALL=0
    for CAPSEM_APT_PACKAGE in $CAPSEM_APT_BASE_PACKAGES; do
        if ! dpkg-query -W -f='${Status}' "$CAPSEM_APT_PACKAGE" 2>/dev/null \
            | grep -q '^install ok installed$'; then
            CAPSEM_APT_NEEDS_INSTALL=1
            break
        fi
    done
    if [ "$CAPSEM_APT_NEEDS_INSTALL" -eq 0 ]; then
        python3 "$CAPSEM_APT_PROJECT_ROOT/scripts/provision-linux-workspace.py" --verify
        printf "  [ok]   Linux system packages\n"
        return 0
    fi

    if ! capsem_linux_confirm "required Linux system packages" "$CAPSEM_APT_ASSUME_YES"; then
        printf "  [FAIL] required Linux system packages were declined\n" >&2
        return 1
    fi

    capsem_linux_as_root env DEBIAN_FRONTEND=noninteractive apt-get update
    capsem_linux_as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y \
        --no-install-recommends \
        $CAPSEM_APT_BASE_PACKAGES
    python3 "$CAPSEM_APT_PROJECT_ROOT/scripts/provision-linux-workspace.py" --verify
    printf "  [ok]   Linux system packages installed\n"
}

capsem_linux_install_dnf_packages() {
    CAPSEM_DNF_PROJECT_ROOT=$1
    CAPSEM_DNF_ASSUME_YES=$2
    CAPSEM_DNF_BINFMT_PACKAGE=$(capsem_linux_dnf_binfmt_package)
    CAPSEM_DNF_WORKSPACE_PACKAGES=$(python3 \
        "$CAPSEM_DNF_PROJECT_ROOT/scripts/provision-linux-workspace.py" --packages dnf)
    CAPSEM_DNF_PACKAGES="
        acl
        cpio
        docker
        docker-buildx-plugin
        python3
        sqlite
        util-linux
        xz
        zstd
        $CAPSEM_DNF_WORKSPACE_PACKAGES
        $CAPSEM_DNF_BINFMT_PACKAGE
    "

    CAPSEM_DNF_NEEDS_INSTALL=0
    for CAPSEM_DNF_PACKAGE in $CAPSEM_DNF_PACKAGES; do
        if ! rpm -q "$CAPSEM_DNF_PACKAGE" >/dev/null 2>&1; then
            CAPSEM_DNF_NEEDS_INSTALL=1
            break
        fi
    done
    if [ "$CAPSEM_DNF_NEEDS_INSTALL" -eq 0 ]; then
        python3 "$CAPSEM_DNF_PROJECT_ROOT/scripts/provision-linux-workspace.py" --verify
        printf "  [ok]   Linux system packages\n"
        return 0
    fi
    if ! capsem_linux_confirm "required Linux system packages" "$CAPSEM_DNF_ASSUME_YES"; then
        printf "  [FAIL] required Linux system packages were declined\n" >&2
        return 1
    fi
    capsem_linux_as_root dnf install -y $CAPSEM_DNF_PACKAGES
    python3 "$CAPSEM_DNF_PROJECT_ROOT/scripts/provision-linux-workspace.py" --verify
    printf "  [ok]   Linux system packages installed\n"
}

capsem_linux_install_node() {
    CAPSEM_NODE_PROJECT_ROOT=$1
    CAPSEM_NODE_ASSUME_YES=$2
    CAPSEM_NODE_MAJOR=$(capsem_linux_node_major \
        "$CAPSEM_NODE_PROJECT_ROOT/config/docker/image/build.toml")
    CAPSEM_NODE_INSTALL_ROOT="$HOME/.local/share/capsem/node"
    CAPSEM_NODE_COMMAND=$(command -v node 2>/dev/null || true)
    CAPSEM_NODE_CURRENT_MAJOR=""
    if [ -n "$CAPSEM_NODE_COMMAND" ]; then
        CAPSEM_NODE_CURRENT_MAJOR=$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || true)
    fi
    CAPSEM_NODE_IS_MANAGED=0
    if [ "$CAPSEM_NODE_COMMAND" = "$HOME/.local/bin/node" ] \
        && [ -L "$HOME/.local/bin/node" ]; then
        case "$(readlink "$HOME/.local/bin/node")" in
            "$CAPSEM_NODE_INSTALL_ROOT"/*) CAPSEM_NODE_IS_MANAGED=1 ;;
        esac
    fi
    if [ "$CAPSEM_NODE_CURRENT_MAJOR" = "$CAPSEM_NODE_MAJOR" ] \
        && [ "$CAPSEM_NODE_IS_MANAGED" -eq 1 ]; then
        printf "  [ok]   Node.js %s (required major %s)\n" \
            "$(node --version)" "$CAPSEM_NODE_MAJOR"
        return 0
    fi

    if ! capsem_linux_confirm \
        "Node.js $CAPSEM_NODE_MAJOR (verified official Linux archive)" \
        "$CAPSEM_NODE_ASSUME_YES"; then
        printf "  [FAIL] Node.js major %s is required; found %s\n" \
            "$CAPSEM_NODE_MAJOR" "${CAPSEM_NODE_CURRENT_MAJOR:-none}" >&2
        return 1
    fi

    case "$(uname -m)" in
        x86_64|amd64) CAPSEM_NODE_ARCH=x64 ;;
        aarch64|arm64) CAPSEM_NODE_ARCH=arm64 ;;
        *)
            printf "  [FAIL] unsupported Linux architecture for Node.js: %s\n" "$(uname -m)" >&2
            return 1 ;;
    esac

    CAPSEM_NODE_TMPDIR=$(mktemp -d "${TMPDIR:-/tmp}/capsem-node.XXXXXX")
    CAPSEM_NODE_SUMS="$CAPSEM_NODE_TMPDIR/SHASUMS256.txt"
    curl --proto '=https' --tlsv1.2 -fsSL \
        "https://nodejs.org/dist/latest-v${CAPSEM_NODE_MAJOR}.x/SHASUMS256.txt" \
        -o "$CAPSEM_NODE_SUMS"
    CAPSEM_NODE_ARCHIVE_NAME=$(awk -v arch="$CAPSEM_NODE_ARCH" '
        $2 ~ ("^node-v[0-9.]+-linux-" arch "[.]tar[.]xz$") { print $2; exit }
    ' "$CAPSEM_NODE_SUMS")
    if [ -z "$CAPSEM_NODE_ARCHIVE_NAME" ]; then
        printf "  [FAIL] Node.js checksum manifest has no Linux %s archive\n" \
            "$CAPSEM_NODE_ARCH" >&2
        rm -f "$CAPSEM_NODE_SUMS"
        rmdir "$CAPSEM_NODE_TMPDIR"
        return 1
    fi

    CAPSEM_NODE_ARCHIVE="$CAPSEM_NODE_TMPDIR/$CAPSEM_NODE_ARCHIVE_NAME"
    curl --proto '=https' --tlsv1.2 -fsSL \
        "https://nodejs.org/dist/latest-v${CAPSEM_NODE_MAJOR}.x/$CAPSEM_NODE_ARCHIVE_NAME" \
        -o "$CAPSEM_NODE_ARCHIVE"
    CAPSEM_NODE_EXPECTED_SHA=$(awk -v archive="$CAPSEM_NODE_ARCHIVE_NAME" \
        '$2 == archive { print $1; exit }' "$CAPSEM_NODE_SUMS")
    CAPSEM_NODE_ACTUAL_SHA=$(sha256sum "$CAPSEM_NODE_ARCHIVE" | awk '{print $1}')
    if [ -z "$CAPSEM_NODE_EXPECTED_SHA" ] \
        || [ "$CAPSEM_NODE_ACTUAL_SHA" != "$CAPSEM_NODE_EXPECTED_SHA" ]; then
        printf "  [FAIL] Node.js archive SHA256 does not match the official manifest\n" >&2
        rm -f "$CAPSEM_NODE_ARCHIVE" "$CAPSEM_NODE_SUMS"
        rmdir "$CAPSEM_NODE_TMPDIR"
        return 1
    fi

    CAPSEM_NODE_TREE=${CAPSEM_NODE_ARCHIVE_NAME%.tar.xz}
    mkdir -p "$CAPSEM_NODE_INSTALL_ROOT" "$HOME/.local/bin"
    tar -xJf "$CAPSEM_NODE_ARCHIVE" -C "$CAPSEM_NODE_INSTALL_ROOT"
    CAPSEM_NODE_BIN="$CAPSEM_NODE_INSTALL_ROOT/$CAPSEM_NODE_TREE/bin"
    for CAPSEM_NODE_TOOL in node npm npx corepack; do
        [ -e "$CAPSEM_NODE_BIN/$CAPSEM_NODE_TOOL" ] || continue
        CAPSEM_NODE_LINK="$HOME/.local/bin/$CAPSEM_NODE_TOOL"
        if [ -L "$CAPSEM_NODE_LINK" ]; then
            case "$(readlink "$CAPSEM_NODE_LINK")" in
                "$CAPSEM_NODE_INSTALL_ROOT"/*) ;;
                *)
                    printf "  [FAIL] refusing to replace unmanaged symlink %s\n" \
                        "$CAPSEM_NODE_LINK" >&2
                    return 1 ;;
            esac
        elif [ -e "$CAPSEM_NODE_LINK" ]; then
            printf "  [FAIL] refusing to replace unmanaged file %s\n" \
                "$CAPSEM_NODE_LINK" >&2
            return 1
        fi
        ln -sfn "$CAPSEM_NODE_BIN/$CAPSEM_NODE_TOOL" "$CAPSEM_NODE_LINK"
    done
    rm -f "$CAPSEM_NODE_ARCHIVE" "$CAPSEM_NODE_SUMS"
    rmdir "$CAPSEM_NODE_TMPDIR"
    hash -r 2>/dev/null || true

    CAPSEM_NODE_INSTALLED_MAJOR=$(node -p 'process.versions.node.split(".")[0]')
    if [ "$CAPSEM_NODE_INSTALLED_MAJOR" != "$CAPSEM_NODE_MAJOR" ]; then
        printf "  [FAIL] installed Node.js major %s, expected %s\n" \
            "$CAPSEM_NODE_INSTALLED_MAJOR" "$CAPSEM_NODE_MAJOR" >&2
        return 1
    fi
    printf "  [ok]   Node.js %s installed from verified official archive\n" \
        "$(node --version)"
}

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
        python3 "$CAPSEM_BUBBLEWRAP_PROJECT_ROOT/scripts/prepare-linux-sandbox.py" --repair-hosted-runner
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
    CAPSEM_BOOTSTRAP_USER=$(capsem_linux_bootstrap_user)
    if command -v modprobe >/dev/null 2>&1; then
        # A stale /dev/vhost-vsock node can outlive its module and makes an
        # existence check lie. Module loading is idempotent; do it before
        # inspecting either device so sysfs and the character node agree.
        capsem_linux_as_root modprobe kvm || true
        capsem_linux_as_root modprobe vhost_vsock || true
    fi

    # systemd-logind's stock uaccess rule can remove a manually applied ACL
    # after the first KVM lifecycle. Install a later rule that keeps both VM
    # devices group-owned and outside that transient seat policy, then apply it
    # before adding the current-session ACL below.
    if command -v udevadm >/dev/null 2>&1; then
        capsem_linux_as_root tee /etc/udev/rules.d/99-capsem-vm-devices.rules \
            >/dev/null <<'EOF'
KERNEL=="kvm", GROUP="kvm", MODE="0666", TAG-="uaccess"
KERNEL=="vhost-vsock", GROUP="kvm", MODE="0660", TAG-="uaccess"
EOF
        capsem_linux_as_root udevadm control --reload-rules
        if [ -e /dev/kvm ]; then
            capsem_linux_as_root udevadm trigger --name-match=kvm
        fi
        if [ -e /dev/vhost-vsock ]; then
            capsem_linux_as_root udevadm trigger --name-match=vhost-vsock
        fi
    else
        printf "  [WARN] udevadm not found; VM device ACLs cannot be made durable\n"
    fi

    if [ "$(id -u "$CAPSEM_BOOTSTRAP_USER")" -ne 0 ] \
        && getent group kvm >/dev/null 2>&1; then
        capsem_linux_as_root usermod -aG kvm "$CAPSEM_BOOTSTRAP_USER"
    fi

    if [ -e /dev/kvm ]; then
        # logind removes named KVM ACLs when the first VM lifecycle changes the
        # device. Match the release CI mode so a second VM in this shell stays
        # usable; KVM itself does not grant raw host-memory access.
        capsem_linux_as_root chmod 0666 /dev/kvm
    fi
    if [ -e /dev/vhost-vsock ]; then
        capsem_linux_as_root setfacl -m "u:$CAPSEM_BOOTSTRAP_USER:rw" /dev/vhost-vsock
    fi

    capsem_linux_verify_vm_devices
}

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
        capsem_linux_prepare_vm_devices
    fi
}
