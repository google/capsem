#!/bin/sh
# Repair Linux KVM device nodes for local Capsem development.
set -eu

if [ "$(uname -s)" != "Linux" ]; then
    echo "KVM device repair is Linux-only" >&2
    exit 1
fi

run_root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    else
        sudo "$@"
    fi
}

misc_minor() {
    awk -v name="$1" '$2 == name { print $1; found = 1 } END { exit found ? 0 : 1 }' /proc/misc
}

ensure_misc_node() {
    name="$1"
    path="$2"
    minor="$(misc_minor "$name")"
    if [ ! -e "$path" ]; then
        run_root mknod "$path" c 10 "$minor"
    fi
    run_root chown root:kvm "$path"
    # Use 0666 for dev bootstrap so the current shell works before group
    # membership is refreshed by a new login session.
    run_root chmod 0666 "$path"
}

if ! grep -Eq '(^flags|^Features)[[:space:]]*:.*\b(vmx|svm)\b' /proc/cpuinfo; then
    echo "CPU virtualization flags vmx/svm are not visible; cannot enable KVM here" >&2
    exit 1
fi

run_root groupadd -f kvm
run_root modprobe kvm
run_root modprobe kvm_intel 2>/dev/null || run_root modprobe kvm_amd 2>/dev/null || true
run_root modprobe vhost_vsock

ensure_misc_node kvm /dev/kvm
ensure_misc_node vhost-vsock /dev/vhost-vsock

target_user="${SUDO_USER:-${USER:-}}"
if [ -n "$target_user" ] && getent passwd "$target_user" >/dev/null 2>&1; then
    run_root usermod -aG kvm "$target_user"
fi

udev_rule='KERNEL=="kvm", GROUP="kvm", MODE="0666", OPTIONS+="static_node=kvm"
KERNEL=="vhost-vsock", GROUP="kvm", MODE="0666", OPTIONS+="static_node=vhost-vsock"'
printf '%s\n' "$udev_rule" | run_root tee /etc/udev/rules.d/99-capsem-kvm.rules >/dev/null

if command -v udevadm >/dev/null 2>&1; then
    run_root udevadm control --reload-rules 2>/dev/null || true
    run_root udevadm trigger --name-match=kvm 2>/dev/null || true
    run_root udevadm trigger --name-match=vhost-vsock 2>/dev/null || true
fi

echo "KVM devices ready: /dev/kvm and /dev/vhost-vsock"
