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
        node)
            case "$pkg" in
                apt) echo "sudo apt install nodejs npm" ;;
                dnf) echo "sudo dnf install nodejs npm" ;;
                *)   echo "https://nodejs.org" ;;
            esac ;;
        pnpm)      echo "npm i -g pnpm" ;;
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
        minisign)
            case "$pkg" in
                apt) echo "sudo apt install minisign" ;;
                dnf) echo "sudo dnf install minisign" ;;
                *)   echo "install minisign via your OS package manager" ;;
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
                apt) echo "sudo apt install docker-buildx-plugin" ;;
                dnf) echo "sudo dnf install docker-buildx-plugin" ;;
                *)   echo "install docker-buildx-plugin" ;;
            esac ;;
        pkg-config)
            case "$pkg" in
                apt) echo "sudo apt install pkg-config libssl-dev libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev" ;;
                dnf) echo "sudo dnf install pkgconf-pkg-config openssl-devel gtk3-devel webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel libxdo-devel" ;;
                *)   echo "install pkg-config, OpenSSL, GTK, WebKitGTK, appindicator, librsvg, and xdo development headers" ;;
            esac ;;
    esac
}

check_platform() {
    section "Platform (Linux)"

    if grep -Eq '(^flags|^Features)[[:space:]]*:.*\b(vmx|svm)\b' /proc/cpuinfo; then
        pass "CPU virtualization flags (vmx/svm)"
    else
        fail "CPU virtualization flags missing -- enable nested virtualization or use a KVM-capable host"
    fi

    if [[ -r /proc/misc ]] && grep -Eq '^[[:space:]]*[0-9]+[[:space:]]+kvm$' /proc/misc; then
        pass "KVM misc device registered"
    else
        fixable linux-kvm-devices "KVM misc device not registered -- load kvm module and create /dev/kvm"
    fi

    if [[ -e /dev/kvm ]]; then
        if [[ -r /dev/kvm ]] && [[ -w /dev/kvm ]]; then
            pass "/dev/kvm (accessible)"
        else
            fixable linux-kvm-devices "/dev/kvm exists but not accessible -- repair permissions and kvm group"
        fi
    else
        fixable linux-kvm-devices "/dev/kvm not found -- create KVM device node"
    fi

    if [[ -r /proc/misc ]] && grep -Eq '^[[:space:]]*[0-9]+[[:space:]]+vhost-vsock$' /proc/misc; then
        pass "vhost-vsock misc device registered"
    else
        fixable linux-kvm-devices "vhost-vsock misc device not registered -- load vhost_vsock module"
    fi

    if [[ -e /dev/vhost-vsock ]]; then
        if [[ -r /dev/vhost-vsock ]] && [[ -w /dev/vhost-vsock ]]; then
            pass "/dev/vhost-vsock (accessible)"
        else
            fixable linux-kvm-devices "/dev/vhost-vsock exists but not accessible -- repair permissions"
        fi
    else
        fixable linux-kvm-devices "/dev/vhost-vsock not found -- create vhost-vsock device node"
    fi

    if command -v pkg-config >/dev/null 2>&1 &&
        pkg-config --exists openssl gtk+-3.0 webkit2gtk-4.1 ayatana-appindicator3-0.1 librsvg-2.0 &&
        [[ -f /usr/include/xdo.h ]]; then
        pass "Linux host-build development headers"
    else
        fixable linux-host-build-deps "Linux host-build development headers missing -- install: $(tool_hint pkg-config)"
    fi

    skip "codesigning (macOS-only, Linux uses KVM)"
}
