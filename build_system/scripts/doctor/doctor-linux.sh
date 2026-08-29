#!/bin/bash
# Capsem Doctor -- Linux-specific checks
# Sourced by doctor-common.sh, do not run directly.

tool_hint() {
    local pkg=""
    if command -v apt-get &>/dev/null; then pkg="apt"; fi
    if command -v dnf &>/dev/null; then pkg="dnf"; fi

    case "$1" in
        rustup)    echo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" ;;
        cargo)     echo "installed with rustup" ;;
        just)      echo "cargo install just" ;;
        node)      echo "run ./bootstrap.sh (installs the configured Node major with SHA256 verification)" ;;
        pnpm)      echo "run ./bootstrap.sh (installs pnpm 10 through the configured Node runtime)" ;;
        python3)
            case "$pkg" in
                apt) echo "sudo apt install python3 python3-venv" ;;
                dnf) echo "sudo dnf install python3" ;;
                *)   echo "https://python.org" ;;
            esac ;;
        uv)        echo "curl -LsSf https://astral.sh/uv/install.sh | sh" ;;
        sqlite3)
            case "$pkg" in
                apt) echo "sudo apt install sqlite3" ;;
                dnf) echo "sudo dnf install sqlite" ;;
                *)   echo "install sqlite3" ;;
            esac ;;
        git)
            case "$pkg" in
                apt) echo "sudo apt install git" ;;
                dnf) echo "sudo dnf install git" ;;
                *)   echo "https://git-scm.com" ;;
            esac ;;
        b3sum)     echo "cargo install b3sum --locked" ;;
        zstd)
            case "$pkg" in
                apt) echo "sudo apt install zstd" ;;
                dnf) echo "sudo dnf install zstd" ;;
                *)   echo "install zstd" ;;
            esac ;;
        cpio)
            case "$pkg" in
                apt) echo "sudo apt install cpio" ;;
                dnf) echo "sudo dnf install cpio" ;;
                *)   echo "install cpio" ;;
            esac ;;
        docker)
            case "$pkg" in
                apt) echo "sudo apt install docker.io" ;;
                dnf) echo "sudo dnf install docker" ;;
                *)   echo "install docker" ;;
            esac ;;
        docker-daemon) echo "start Docker: sudo systemctl start docker" ;;
        docker-buildx)
            case "$pkg" in
                apt) echo "sudo apt install docker-buildx" ;;
                dnf) echo "sudo dnf install docker-buildx-plugin" ;;
                *)   echo "install docker-buildx-plugin" ;;
            esac ;;
        bwrap)
            case "$pkg" in
                apt) echo "sudo apt install bubblewrap" ;;
                dnf) echo "sudo dnf install bubblewrap" ;;
                *)   echo "install bubblewrap" ;;
            esac ;;
        musl-tools)
            case "$pkg" in
                apt) echo "sudo apt install musl-tools" ;;
                dnf) echo "sudo dnf install musl-gcc" ;;
                *)   echo "install musl-gcc / musl-tools" ;;
            esac ;;
    esac
}

_doctor_sudo() {
    if [[ "$(id -u)" -eq 0 ]]; then
        "$@"
    else
        sudo "$@"
    fi
}

_doctor_install_linux_musl_tools() {
    if command -v apt-get &>/dev/null; then
        _doctor_sudo env DEBIAN_FRONTEND=noninteractive apt-get update
        _doctor_sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y musl-tools
    elif command -v dnf &>/dev/null; then
        _doctor_sudo dnf install -y musl-gcc
    else
        echo "No supported package manager found. Install musl-gcc / musl-tools manually." >&2
        return 1
    fi
}

linux_musl_toolchain_available() {
    command -v musl-gcc >/dev/null 2>&1 \
        && musl-gcc --version >/dev/null 2>&1
}

check_linux_musl_toolchain() {
    section "C Toolchain"

    if linux_musl_toolchain_available; then
        pass "musl-gcc"
    else
        fixable linux-musl-tools "musl-gcc missing -- install: $(tool_hint musl-tools)"
    fi
}

check_platform() {
    local interfaces sandbox_policy sandbox_policy_valid
    section "Platform (Linux)"

    sandbox_policy="${CAPSEM_GATE_COMMAND_SANDBOX_MODE:-}"
    sandbox_policy_valid=1
    case "$sandbox_policy" in
        ""|off|enforce) ;;
        report)
            fail "Linux gate sandbox report mode is unsupported"
            sandbox_policy_valid=0
            ;;
        *)
            fail "unknown gate sandbox policy: $sandbox_policy"
            sandbox_policy_valid=0
            ;;
    esac

    if ! command -v bwrap &>/dev/null; then
        fail "Bubblewrap not found -- install: $(tool_hint bwrap); or run ./bootstrap.sh"
    elif [[ "$sandbox_policy_valid" -eq 0 ]]; then
        :
    elif [[ "$sandbox_policy" == "enforce" ]]; then
        interfaces=$(capsem_linux_network_interfaces \
            | tr '\n' ' ' | sed 's/ $//')
        if [[ "$interfaces" == "lo" ]]; then
            pass "Bubblewrap gate network namespace active (loopback only)"
        else
            fail "enforcing gate sandbox sees interfaces: ${interfaces:-unknown}"
        fi
    elif bwrap --unshare-net --die-with-parent --new-session \
        --bind / / --dev-bind /dev /dev -- sh -c ': > /dev/null' \
        >/dev/null 2>&1; then
        pass "Bubblewrap gate network namespace and device mount"
    else
        fail "Bubblewrap cannot create a usable gate namespace -- run ./bootstrap.sh"
    fi

    # KVM and its guest/host communication transport are one runtime boundary.
    # A readable /dev/kvm alone can create a VM that immediately dies opening
    # vhost-vsock, so report and repair them as one pair.
    if [[ -n "${CAPSEM_SKIP_KVM_CHECK:-}" ]]; then
        for device in /dev/kvm /dev/vhost-vsock; do
            skip "$device (CAPSEM_SKIP_KVM_CHECK set)"
        done
    else
        for device in /dev/kvm /dev/vhost-vsock; do
            if [[ -e "$device" ]] && [[ -r "$device" ]] && [[ -w "$device" ]]; then
                pass "$device (accessible)"
            elif [[ -e "$device" ]]; then
                fail "$device exists but is not accessible -- run ./bootstrap.sh"
            else
                warn "$device not found -- VM features require it; run ./bootstrap.sh"
            fi
        done
    fi

    skip "codesigning (macOS-only, Linux uses KVM)"
}
