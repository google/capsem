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


def test_python_venv_active():
    """A Python venv must be activated so pip install works on read-only rootfs."""
    venv = os.environ.get("VIRTUAL_ENV")
    assert venv and os.path.isdir(venv), (
        f"VIRTUAL_ENV not set or missing (got {venv!r}). "
        "pip install will fail on the read-only rootfs without an active venv."
    )


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


def test_root_is_writable_filesystem():
    """/root must be mounted as a writable filesystem (ext4 or virtiofs)."""
    result = run("mount | grep 'on /root '")
    assert result.returncode == 0, "/root not mounted"
    mount_info = result.stdout.strip()
    assert "ext4" in mount_info or "virtiofs" in mount_info, \
        f"/root is not ext4 or virtiofs: {mount_info}"


def test_root_workspace_writable():
    """/root must be writable (create + read + delete a file)."""
    result = run("echo capsem_test > /root/.write_test && cat /root/.write_test && rm /root/.write_test")
    assert result.returncode == 0, "/root is not writable"
    assert "capsem_test" in result.stdout


def test_tmp_is_writable():
    """/tmp must be writable (writes go through overlayfs to tmpfs upper)."""
    test_file = "/tmp/.capsem_write_test"
    result = run(f'echo "writable" > {test_file} && cat {test_file}')
    assert result.returncode == 0, "/tmp is not writable"
    assert "writable" in result.stdout
    run(f"rm -f {test_file}")


def test_rootfs_is_overlay():
    """Root filesystem must be an overlay mount."""
    result = run("mount | grep 'on / '")
    assert result.returncode == 0, "root mount not found"
    assert "overlay" in result.stdout, f"/ is not overlay: {result.stdout}"


def test_virtiofs_kernel_support():
    """Kernel must have virtiofs support (needed for VirtioFS storage mode)."""
    result = run("cat /proc/filesystems")
    assert result.returncode == 0, "/proc/filesystems not readable"
    assert "virtiofs" in result.stdout, \
        "virtiofs not in /proc/filesystems -- kernel missing CONFIG_VIRTIO_FS"
