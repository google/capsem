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
        flock)
            case "$pkg" in
                apt) echo "sudo apt install util-linux" ;;
                dnf) echo "sudo dnf install util-linux" ;;
                *)   echo "install util-linux (provides flock)" ;;
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
    esac
}

check_platform() {
    section "Platform (Linux)"

    # KVM
    if [[ -e /dev/kvm ]]; then
        if [[ -r /dev/kvm ]] && [[ -w /dev/kvm ]]; then
            pass "/dev/kvm (accessible)"
        else
            fail "/dev/kvm exists but not accessible -- fix: sudo usermod -aG kvm $USER"
        fi
    else
        warn "/dev/kvm not found -- VM features require KVM"
    fi

    skip "codesigning (macOS-only, Linux uses KVM)"
}
