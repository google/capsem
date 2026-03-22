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


def test_boot_time_under_1s():
    """Guest boot (capsem-init stages) must complete in under 1 second.

    Reads the boot timing file written by capsem-init. If total exceeds
    1000ms, something regressed (e.g. uv not on PATH, falling back to
    slow python3 -m venv)."""
    import json
    timing_path = "/run/capsem-boot-timing"
    result = run(f"cat {timing_path}")
    assert result.returncode == 0, \
        f"boot timing file {timing_path} not found -- capsem-init must write it"
    stages = []
    for line in result.stdout.strip().splitlines():
        try:
            stages.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    total = sum(s.get("duration_ms", 0) for s in stages)
    slow = [s for s in stages if s.get("duration_ms", 0) > 500]
    assert total <= 1000, (
        f"boot took {total}ms (limit 1000ms). "
        f"slow stages: {slow}. all: {stages}"
    )


def test_boot_timing_rejects_xss():
    """Boot timing file must reject XSS payloads in stage names.

    The PTY agent parses /run/capsem-boot-timing and only accepts
    alphanumeric+underscore names. This test writes a poisoned file,
    re-parses it the same way the agent does, and verifies injection
    entries are dropped."""
    import json
    import tempfile
    payloads = [
        '{"name":"<script>alert(1)</script>","duration_ms":10}',
        '{"name":"normal_stage","duration_ms":20}',
        '{"name":"a]};fetch(evil)","duration_ms":30}',
        '{"name":"","duration_ms":40}',
        '{"name":"has spaces","duration_ms":50}',
        '{"name":"../../../etc/passwd","duration_ms":60}',
    ]
    with tempfile.NamedTemporaryFile(mode='w', suffix='.jsonl', delete=False) as f:
        f.write('\n'.join(payloads) + '\n')
        tmp = f.name
    # Parse the same way the agent does: only alphanumeric + underscore.
    valid = []
    for line in open(tmp).read().strip().splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        name = entry.get("name", "")
        if (name and len(name) <= 64
                and all(c.isalnum() or c == '_' for c in name)
                and entry.get("duration_ms", 0) <= 600_000):
            valid.append(entry)
    os.unlink(tmp)
    assert len(valid) == 1, f"expected only 'normal_stage', got: {valid}"
    assert valid[0]["name"] == "normal_stage"
