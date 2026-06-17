"""VirtioFS storage mode tests.

These tests verify the VirtioFS single-share hybrid architecture:
- ext4 on virtio-blk (/dev/vdb) for overlayfs upper (system packages)
- direct VirtioFS workspace for /root (AI workspace)
- file write/read through the full stack
"""

import os
import pathlib
import textwrap
import zipfile

import pytest

from conftest import run


def is_virtiofs_mode():
    """Check if the VM booted in VirtioFS mode."""
    result = run("mount | grep 'on /root '")
    return "virtiofs" in result.stdout


@pytest.fixture(autouse=True)
def virtiofs_only():
    """Require VirtioFS mode for this storage contract."""
    if not is_virtiofs_mode():
        pytest.fail("not in VirtioFS mode")


def test_virtiofs_root_mount():
    """/root must be mounted via VirtioFS (bind-mount from workspace)."""
    result = run("mount | grep 'on /root '")
    assert result.returncode == 0, "/root not mounted"
    assert "virtiofs" in result.stdout, f"/root not virtiofs: {result.stdout}"


def test_overlayfs_with_virtio_blk_upper():
    """Root overlay must be a stacked overlay (capsem-init pivot_root drops
    the upper-mount path from the post-chroot namespace, but the overlay
    itself is reported on / via the kernel's stacked mount info)."""
    result = run("mount | grep 'on / '")
    assert "overlay" in result.stdout, f"/ not overlay: {result.stdout}"


def test_system_overlay_block_device_present():
    """The system-overlay virtio-blk device (/dev/vdb) must be attached
    and usable as an ext4 device. capsem-init mounts /dev/vdb pre-chroot
    so it isn't visible in `mount` after switch_root, but the device node
    survives in /sys/class/block."""
    result = run("[ -b /dev/vdb ] && echo present || echo absent")
    assert "present" in result.stdout, f"/dev/vdb not a block device: {result.stdout}"
    # Confirm it really is the ext4 system overlay (magic 0xEF53 at offset 0x438).
    result = run("tail -c +1081 /dev/vdb 2>/dev/null | head -c 2 | od -A n -t x1")
    assert "53 ef" in result.stdout.lower(), f"/dev/vdb not ext4-formatted: {result.stdout!r}"


def test_storage_capacity_report_is_available():
    """Doctor must surface block and inode availability for storage triage."""
    block_result = run("df -h / /root /tmp")
    assert block_result.returncode == 0, f"df -h failed: {block_result.stdout}\n{block_result.stderr}"
    assert "/root" in block_result.stdout, f"df -h missing /root row: {block_result.stdout}"

    inode_result = run("df -i / /root /tmp")
    assert inode_result.returncode == 0, f"df -i failed: {inode_result.stdout}\n{inode_result.stderr}"
    assert "IUse%" in inode_result.stdout, f"df -i missing inode utilization: {inode_result.stdout}"


def test_overlay_mount_options_are_reported():
    """Doctor must expose overlay mount options when package writes fail."""
    result = run("awk '$2 == \"/\" && $3 == \"overlay\" { print $4 }' /proc/mounts")
    assert result.returncode == 0
    assert result.stdout.strip(), f"overlay mount options missing from /proc/mounts: {result.stdout}"
    assert "upperdir=" in result.stdout, f"overlay options missing upperdir: {result.stdout}"


def test_workspace_write_read():
    """Write a file to /root and read it back."""
    test_file = pathlib.Path("/root/virtiofs_write_test.txt")
    content = "VirtioFS write test from capsem-doctor"
    test_file.write_text(content)
    assert test_file.read_text() == content
    test_file.unlink()


def test_workspace_large_file():
    """Write a 1MB file to /root workspace and verify size."""
    test_file = pathlib.Path("/root/virtiofs_large_test.bin")
    result = run(f"dd if=/dev/urandom of={test_file} bs=1K count=1024 2>&1")
    assert result.returncode == 0
    assert os.path.getsize(test_file) == 1024 * 1024
    test_file.unlink()


def test_workspace_subdirectory():
    """Create a nested directory structure in /root workspace."""
    base = pathlib.Path("/root/virtiofs_test_dir/sub1/sub2")
    base.mkdir(parents=True, exist_ok=True)
    test_file = base / "nested.txt"
    test_file.write_text("nested content")
    assert test_file.read_text() == "nested content"
    # Cleanup
    import shutil
    shutil.rmtree("/root/virtiofs_test_dir")


def test_system_overlay_writable():
    """System overlay (ext4 on /dev/vdb) must be writable for package installs."""
    # Write to a system path (goes through overlayfs -> ext4 virtio-blk upper).
    test_file = pathlib.Path("/tmp/overlay_write_test.txt")
    test_file.write_text("overlay write test")
    assert test_file.read_text() == "overlay write test"
    test_file.unlink()


def _write_python_wheel(output_dir, distribution, module, module_source):
    """Create a tiny pure-Python wheel without touching a package index."""
    version = "0.1.0"
    normalized = distribution.replace("-", "_")
    wheel_path = output_dir / f"{normalized}-{version}-py3-none-any.whl"
    dist_info = f"{normalized}-{version}.dist-info"
    files = {
        f"{module}/__init__.py": textwrap.dedent(module_source).lstrip(),
        f"{dist_info}/METADATA": (
            "Metadata-Version: 2.1\n"
            f"Name: {distribution}\n"
            f"Version: {version}\n"
        ),
        f"{dist_info}/WHEEL": (
            "Wheel-Version: 1.0\n"
            "Generator: capsem-doctor\n"
            "Root-Is-Purelib: true\n"
            "Tag: py3-none-any\n"
        ),
    }
    record_rows = [f"{path},," for path in files]
    record_rows.append(f"{dist_info}/RECORD,,")
    files[f"{dist_info}/RECORD"] = "\n".join(record_rows) + "\n"
    with zipfile.ZipFile(wheel_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for path, data in files.items():
            zf.writestr(path, data)
    return wheel_path


def test_pip_install_works(output_dir):
    """pip install must work (writes to ext4 virtio-blk overlay, not VirtioFS)."""
    wheel = _write_python_wheel(
        output_dir,
        "capsem-virtiofs-pip",
        "capsem_virtiofs_pip",
        """
        def moo():
            return "moo"
        """,
    )
    result = run(f"pip install --no-index {wheel} 2>&1", timeout=30)
    assert result.returncode == 0, f"pip install failed: {result.stdout}\n{result.stderr}"
    result = run("python3 -c 'import capsem_virtiofs_pip; print(capsem_virtiofs_pip.moo())'")
    assert result.returncode == 0, f"local wheel not importable: {result.stderr}"
    assert "moo" in result.stdout, f"local wheel not working: {result.stdout}"


def test_file_delete_and_recreate():
    """Delete a file in /root and recreate it (tests VirtioFS delete + create)."""
    test_file = pathlib.Path("/root/virtiofs_delete_test.txt")
    test_file.write_text("version1")
    assert test_file.exists()
    test_file.unlink()
    assert not test_file.exists()
    test_file.write_text("version2")
    assert test_file.read_text() == "version2"
    test_file.unlink()
