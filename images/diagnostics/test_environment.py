"""Shell environment, VM configuration, and mount tests."""

import os

import pytest

from conftest import run


# -- Environment variables --


def test_term_is_xterm_256color():
    """TERM must be xterm-256color for proper terminal rendering."""
    assert os.environ.get("TERM") == "xterm-256color", \
        f"TERM={os.environ.get('TERM')}"


def test_home_is_root():
    """HOME must be /root."""
    assert os.environ.get("HOME") == "/root", \
        f"HOME={os.environ.get('HOME')}"


def test_path_includes_standard_dirs():
    """PATH must include /usr/local/bin and /usr/bin."""
    path = os.environ.get("PATH", "")
    assert "/usr/local/bin" in path, f"/usr/local/bin not in PATH: {path}"
    assert "/usr/bin" in path, f"/usr/bin not in PATH: {path}"


# -- Shell --


def test_shell_is_bash():
    """Bash must be installed and executable."""
    result = run("bash --version")
    assert result.returncode == 0 and "bash" in result.stdout.lower(), \
        f"bash not available: {result.stdout.strip()}"


# -- Kernel and architecture --


def test_kernel_is_linux_6():
    """Kernel must be Linux 6.x (custom LTS build)."""
    result = run("uname -r")
    assert result.returncode == 0
    version = result.stdout.strip()
    assert version.startswith("6."), f"unexpected kernel version: {version}"


def test_architecture_is_aarch64():
    """Architecture must be aarch64 (ARM64 on Apple Silicon)."""
    result = run("uname -m")
    assert result.returncode == 0
    assert result.stdout.strip() == "aarch64", \
        f"unexpected arch: {result.stdout.strip()}"


# -- Mount points --


def test_proc_mounted():
    """/proc must be mounted."""
    assert os.path.isdir("/proc"), "/proc not present"
    assert os.path.isfile("/proc/version"), "/proc/version not readable"


def test_sys_mounted():
    """/sys must be mounted."""
    assert os.path.isdir("/sys"), "/sys not present"


def test_dev_mounted():
    """/dev must be mounted (devtmpfs)."""
    assert os.path.isdir("/dev"), "/dev not present"
    assert os.path.exists("/dev/null"), "/dev/null not present"
    assert os.path.exists("/dev/zero"), "/dev/zero not present"
    assert os.path.exists("/dev/urandom"), "/dev/urandom not present"


def test_dev_pts_mounted():
    """/dev/pts must be mounted for PTY support."""
    result = run("mount | grep devpts")
    assert result.returncode == 0, "/dev/pts not mounted"


# -- Tmpfs sizes --


def test_root_is_ext4_scratch_disk():
    """/root must be mounted from the ephemeral scratch disk (ext4)."""
    result = run("mount | grep 'on /root '")
    assert result.returncode == 0, "/root not mounted"
    assert "ext4" in result.stdout, f"/root is not ext4: {result.stdout}"


def test_root_scratch_disk_size():
    """/root scratch disk should be at least 4GB."""
    result = run("df -B1 /root | tail -1 | awk '{print $2}'")
    assert result.returncode == 0, "df failed"
    size_bytes = int(result.stdout.strip())
    # At least 4GB (smallest reasonable scratch disk)
    assert size_bytes >= 4 * 1024 * 1024 * 1024, \
        f"/root scratch disk too small: {size_bytes / (1024**3):.1f} GB"


def test_tmp_is_writable_tmpfs():
    """/tmp must be a writable tmpfs mount."""
    result = run("mount | grep 'on /tmp '")
    assert result.returncode == 0, "/tmp not mounted"
    assert "tmpfs" in result.stdout, f"/tmp is not tmpfs: {result.stdout}"
