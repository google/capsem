"""Sandbox security tests -- validates the VM's isolation model."""

import os
import subprocess
import time

import pytest

import pytest

from conftest import run


# -- Clock synchronization --

def test_clock_is_synchronized():
    """System clock should be within 60 seconds of real time."""
    result = run("date +%s")
    assert result.returncode == 0
    guest_time = int(result.stdout.strip())
    host_time = int(time.time())
    assert abs(guest_time - host_time) < 60, \
        f"clock drift too large: guest={guest_time}, host={host_time}, delta={abs(guest_time - host_time)}s"


# -- Filesystem isolation --

def test_squashfs_is_immutable():
    """The rootfs block device (/dev/vda) must be squashfs (structurally immutable)."""
    # blkid reads the filesystem type directly from the block device,
    # independent of mount visibility from inside the chroot.
    result = run("blkid -o value -s TYPE /dev/vda 2>&1")
    assert result.returncode == 0, f"/dev/vda not found or blkid failed: {result.stdout}"
    assert result.stdout.strip() == "squashfs", \
        f"/dev/vda is not squashfs: {result.stdout}"


def test_overlay_configured():
    """An overlay mount must exist as the root filesystem."""
    result = run("mount | grep 'on / '")
    assert result.returncode == 0, "root mount not found"
    assert "overlay" in result.stdout, f"root is not overlay: {result.stdout}"
    # Verify overlay has lower and upper dirs configured
    result = run("grep ' / overlay ' /proc/mounts")
    assert result.returncode == 0, "overlay not in /proc/mounts"
    assert "lowerdir=" in result.stdout, f"overlay missing lowerdir: {result.stdout}"
    assert "upperdir=" in result.stdout, f"overlay missing upperdir: {result.stdout}"


def test_overlay_writes_are_ephemeral():
    """Writes to system paths succeed through overlay (goes to tmpfs upper, not squashfs)."""
    test_file = "/usr/bin/.capsem_overlay_test"
    result = run(f'echo "overlay-ok" > {test_file} && cat {test_file}')
    assert result.returncode == 0, "write to /usr/bin through overlay failed"
    assert "overlay-ok" in result.stdout
    run(f"rm -f {test_file}")


@pytest.mark.parametrize("path", ["/root", "/tmp", "/run", "/var/log", "/var/tmp"])
def test_writable_mounts(path):
    """Writable paths (/root=ext4 scratch, others=overlay tmpfs upper) must allow write + readback."""
    test_file = f"{path}/.capsem_rw_test"
    payload = "capsem-writable-ok"
    result = run(f'echo "{payload}" > {test_file} && cat {test_file}')
    assert result.returncode == 0
    assert payload in result.stdout
    run(f"rm -f {test_file}")


# -- Binary security --

GUEST_BINARY_PATHS = [
    "/usr/local/bin/capsem-pty-agent",
    "/run/capsem-pty-agent",
    "/usr/local/bin/capsem-net-proxy",
    "/run/capsem-net-proxy",
]


@pytest.fixture(params=GUEST_BINARY_PATHS)
def guest_binary(request):
    """Yield each guest binary path that exists on this guest."""
    path = request.param
    if not os.path.isfile(path):
        pytest.skip(f"{path} not present")
    return path


def test_guest_binary_not_writable(guest_binary):
    """Guest binaries must not be writable (chmod 555)."""
    import stat
    mode = os.stat(guest_binary).st_mode
    writable = mode & (stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH)
    assert writable == 0, f"{guest_binary} has write bits set (mode={oct(mode)})"


def test_guest_binary_executable(guest_binary):
    """Guest binaries must be executable."""
    assert os.access(guest_binary, os.X_OK), f"{guest_binary} is not executable"


# -- No setuid/setgid --

def test_no_setuid_binaries():
    """No setuid binaries should exist in the rootfs."""
    result = run("find / -xdev -perm -4000 -type f 2>/dev/null", timeout=30)
    files = result.stdout.strip()
    assert files == "", f"setuid binaries found:\n{files}"


def test_no_setgid_binaries():
    """No setgid binaries should exist in the rootfs."""
    result = run("find / -xdev -perm -2000 -type f 2>/dev/null", timeout=30)
    files = result.stdout.strip()
    assert files == "", f"setgid binaries found:\n{files}"


# -- Kernel hardening --

def test_no_kernel_modules():
    """Kernel module loading must be disabled (CONFIG_MODULES=n)."""
    result = run("modprobe dummy 2>&1")
    assert result.returncode != 0, "modprobe should fail with CONFIG_MODULES=n"


def test_no_dev_mem():
    """/dev/mem must not exist (CONFIG_DEVMEM=n)."""
    assert not os.path.exists("/dev/mem"), "/dev/mem exists"


def test_no_dev_port():
    """/dev/port must not exist (CONFIG_DEVPORT=n)."""
    assert not os.path.exists("/dev/port"), "/dev/port exists"


def test_no_proc_kcore():
    """/proc/kcore must not be readable."""
    if not os.path.exists("/proc/kcore"):
        return  # not present at all, also fine
    result = run("cat /proc/kcore 2>&1")
    assert result.returncode != 0, "/proc/kcore is readable"


# -- Network isolation (air-gapped SNI proxy) --

def test_dummy_interface_exists():
    """dummy0 interface must exist for air-gapped networking."""
    result = run("ip link show dummy0")
    assert result.returncode == 0, "dummy0 interface not found"


def test_dns_resolves_to_local():
    """All DNS queries must resolve to 10.0.0.1 (fake DNS)."""
    result = run("getent hosts github.com 2>&1", timeout=5)
    assert "10.0.0.1" in result.stdout, f"DNS did not resolve to 10.0.0.1:\n{result.stdout}"


def test_iptables_redirect():
    """iptables REDIRECT rule must capture port 443 to 10443."""
    # Try iptables-legacy first (kernel has NF_TABLES=n), fall back to iptables
    result = run("iptables-legacy -t nat -L -n 2>&1 || iptables -t nat -L -n 2>&1", timeout=5)
    assert "REDIRECT" in result.stdout, f"no REDIRECT rule:\n{result.stdout}"
    assert "10443" in result.stdout, f"no redirect to 10443:\n{result.stdout}"


def test_net_proxy_running():
    """capsem-net-proxy must be running."""
    result = run("pgrep -f capsem-net-proxy")
    assert result.returncode == 0, "capsem-net-proxy is not running"


def test_allowed_domain():
    """HTTPS to an allowed domain -- step-by-step handshake diagnostic."""
    errors = []

    # Step 1: DNS resolves to 10.0.0.1
    r = run("getent hosts elie.net", timeout=5)
    if "10.0.0.1" not in r.stdout:
        errors.append(f"DNS: expected 10.0.0.1, got: {r.stdout.strip()}")

    # Step 2: TCP connect to 10.0.0.1:443 (should be redirected to 10443)
    r = run(
        "python3 -c \""
        "import socket; s=socket.socket(); s.settimeout(5); "
        "s.connect(('10.0.0.1', 443)); "
        "print('TCP_OK'); s.close()\"",
        timeout=10,
    )
    if "TCP_OK" not in r.stdout:
        errors.append(f"TCP connect: {r.stderr.strip() or r.stdout.strip()}")

    # Step 3: TCP connect directly to net-proxy port
    r = run(
        "python3 -c \""
        "import socket; s=socket.socket(); s.settimeout(5); "
        "s.connect(('127.0.0.1', 10443)); "
        "print('PROXY_OK'); s.close()\"",
        timeout=10,
    )
    if "PROXY_OK" not in r.stdout:
        errors.append(f"net-proxy TCP: {r.stderr.strip() or r.stdout.strip()}")

    # Step 4: Send TLS ClientHello and check if we get a ServerHello back
    r = run(
        "python3 -c \""
        "import socket, ssl; "
        "s = socket.socket(); s.settimeout(10); "
        "s.connect(('10.0.0.1', 443)); "
        "ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT); "
        "ctx.check_hostname = False; "
        "ctx.verify_mode = ssl.CERT_NONE; "
        "ws = ctx.wrap_socket(s, server_hostname='elie.net'); "
        "print('TLS_OK version=' + str(ws.version())); "
        "ws.close()\" 2>&1",
        timeout=15,
    )
    if "TLS_OK" not in r.stdout:
        errors.append(f"TLS handshake: {r.stdout.strip()}")

    # Step 5: Full HTTPS request
    r = run("curl -skI --connect-timeout 10 https://elie.net 2>&1", timeout=20)
    if r.returncode != 0:
        errors.append(f"curl exit {r.returncode}: {r.stdout.strip()}")
    elif "HTTP/" not in r.stdout:
        errors.append(f"curl no HTTP response: {r.stdout.strip()}")

    assert not errors, "HTTPS handshake diagnostic:\n" + "\n".join(
        f"  [{i+1}] {e}" for i, e in enumerate(errors)
    )


def test_denied_domain():
    """HTTPS to a denied domain (example.com) must be rejected (403 or refused)."""
    result = run("curl -sI --connect-timeout 5 https://example.com 2>&1", timeout=15)
    assert result.returncode != 0 or "403" in result.stdout, \
        f"curl to denied domain should fail or return 403: {result.stdout}"


def test_no_real_nics():
    """No real network interfaces should exist (only lo and dummy0)."""
    result = run("ls /sys/class/net/ 2>/dev/null")
    if result.returncode != 0:
        return  # /sys/class/net/ doesn't exist, that's fine
    nics = result.stdout.strip().split()
    real_prefixes = ("eth", "wlan", "ens", "enp")
    real_nics = [n for n in nics if n.startswith(real_prefixes)]
    assert real_nics == [], f"real NICs found: {real_nics}"


# -- Process integrity --

def test_pty_agent_running():
    """capsem-pty-agent must be running."""
    result = run("pgrep -f capsem-pty-agent")
    assert result.returncode == 0, "capsem-pty-agent is not running"


def test_dnsmasq_running():
    """dnsmasq must be running for fake DNS."""
    result = run("pgrep dnsmasq")
    assert result.returncode == 0, "dnsmasq is not running"


def test_no_systemd():
    """systemd must not be running (no service manager in the VM)."""
    result = run("pgrep systemd")
    assert result.returncode != 0, "systemd is running but should not be"


def test_no_sshd():
    """sshd must not be running (no remote access to the VM)."""
    result = run("pgrep sshd")
    assert result.returncode != 0, "sshd is running but should not be"


def test_no_cron():
    """cron must not be running (no scheduled tasks in the VM)."""
    result = run("pgrep cron")
    assert result.returncode != 0, "cron is running but should not be"


# -- Additional kernel hardening --

def test_proc_modules_empty():
    """/proc/modules must be empty or absent (CONFIG_MODULES=n)."""
    if not os.path.exists("/proc/modules"):
        return  # file absent means modules are compiled out entirely
    result = run("cat /proc/modules")
    assert result.returncode == 0
    assert result.stdout.strip() == "", f"loaded modules found:\n{result.stdout}"


def test_no_debugfs():
    """debugfs must not be mounted (CONFIG_DEBUG_FS=n)."""
    result = run("mount | grep debugfs")
    assert result.returncode != 0, "debugfs is mounted but should not be"


def test_no_ipv6():
    """IPv6 must be disabled (CONFIG_IPV6=n)."""
    assert not os.path.exists("/proc/net/if_inet6"), \
        "IPv6 is enabled (/proc/net/if_inet6 exists)"


def test_kernel_cmdline_has_ro():
    """Kernel cmdline must include 'ro' for read-only rootfs."""
    result = run("cat /proc/cmdline")
    assert result.returncode == 0
    # Pad with spaces so we match 'ro' as a standalone token
    cmdline = f" {result.stdout.strip()} "
    assert " ro " in cmdline, f"'ro' not in cmdline: {result.stdout}"


def test_swap_active():
    """Swap: active on scratch disk in block mode, absent in VirtioFS mode."""
    mount_result = run("mount | grep 'on /root '")
    is_virtiofs = "virtiofs" in mount_result.stdout
    result = run("cat /proc/swaps")
    assert result.returncode == 0
    swap_lines = [l for l in result.stdout.strip().split('\n') if l.strip()]
    if is_virtiofs:
        # VirtioFS mode: no swap file expected.
        assert len(swap_lines) <= 1, \
            f"swap should not be active in VirtioFS mode:\n{result.stdout}"
    else:
        # Block mode: swap on scratch disk.
        assert len(swap_lines) >= 2, f"swap is not active:\n{result.stdout}"
        assert "/root/.swapfile" in result.stdout, \
            f"swap not on scratch disk:\n{result.stdout}"


def test_loopback_interface_up():
    """Loopback interface must be up."""
    result = run("ip link show lo")
    assert result.returncode == 0
    assert "UP" in result.stdout, "lo interface is not UP"


def test_no_kallsyms():
    """Kernel symbol table must not be exposed (CONFIG_KALLSYMS=n)."""
    if not os.path.exists("/proc/kallsyms"):
        return  # file doesn't exist at all, that's fine
    result = run("wc -l < /proc/kallsyms")
    assert result.returncode == 0
    count = int(result.stdout.strip())
    assert count == 0, f"/proc/kallsyms has {count} symbols (should be empty or absent)"


# -- Kernel cmdline hardening --

def test_init_on_alloc():
    """Kernel cmdline must include init_on_alloc=1 for heap zeroing."""
    result = run("cat /proc/cmdline")
    assert result.returncode == 0
    assert "init_on_alloc=1" in result.stdout, \
        f"init_on_alloc=1 not in cmdline: {result.stdout}"


def test_slab_nomerge():
    """Kernel cmdline must include slab_nomerge for heap isolation."""
    result = run("cat /proc/cmdline")
    assert result.returncode == 0
    assert "slab_nomerge" in result.stdout, \
        f"slab_nomerge not in cmdline: {result.stdout}"


def test_page_alloc_shuffle():
    """Kernel cmdline must include page_alloc.shuffle=1 for page randomization."""
    result = run("cat /proc/cmdline")
    assert result.returncode == 0
    assert "page_alloc.shuffle=1" in result.stdout, \
        f"page_alloc.shuffle=1 not in cmdline: {result.stdout}"


def test_seccomp_available():
    """Seccomp must be available (CONFIG_SECCOMP=y)."""
    result = run("grep '^Seccomp:' /proc/self/status")
    assert result.returncode == 0, \
        "Seccomp line not found in /proc/self/status"
