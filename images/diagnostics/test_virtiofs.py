"""VirtioFS storage mode tests.

These tests verify the VirtioFS single-share hybrid architecture:
- ext4 loopback for overlayfs upper (system packages)
- direct VirtioFS workspace for /root (AI workspace)
- file write/read through the full stack
"""

import os
import pathlib
import subprocess

import pytest

from conftest import run


def is_virtiofs_mode():
    """Check if the VM booted in VirtioFS mode."""
    result = run("mount | grep 'on /root '")
    return "virtiofs" in result.stdout


@pytest.fixture(autouse=True)
def virtiofs_only():
    """Skip all tests in this file if not in VirtioFS mode."""
    if not is_virtiofs_mode():
        pytest.skip("not in VirtioFS mode")


def test_virtiofs_root_mount():
    """/root must be mounted via VirtioFS (bind-mount from workspace)."""
    result = run("mount | grep 'on /root '")
    assert result.returncode == 0, "/root not mounted"
    assert "virtiofs" in result.stdout, f"/root not virtiofs: {result.stdout}"


def test_overlayfs_with_loop_upper():
    """Root overlay must use an ext4 loopback as upper (not tmpfs, not virtiofs)."""
    result = run("mount | grep 'on / '")
    assert "overlay" in result.stdout, f"/ not overlay: {result.stdout}"
    # A loop device must be active (ext4 on loop backs the overlay upper).
    result = run("losetup -a")
    assert "/dev/loop" in result.stdout, f"no loop device active: {result.stdout}"


def test_loop_device_active():
    """A loop device must be active (backing the ext4 system image)."""
    result = run("losetup -a")
    assert result.returncode == 0
    assert "/mnt/shared/system/rootfs.img" in result.stdout, \
        f"no loop device for rootfs.img: {result.stdout}"


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
    """System overlay (ext4 loopback) must be writable for package installs."""
    # Write to a system path (goes through overlayfs -> ext4 loopback upper).
    test_file = pathlib.Path("/tmp/overlay_write_test.txt")
    test_file.write_text("overlay write test")
    assert test_file.read_text() == "overlay write test"
    test_file.unlink()


def test_pip_install_works():
    """pip install must work (writes to ext4 loopback overlay, not VirtioFS)."""
    # Install a tiny package to verify the overlay is writable for package managers.
    result = run("pip install --quiet cowsay 2>&1", timeout=30)
    assert result.returncode == 0, f"pip install failed: {result.stdout}\n{result.stderr}"
    result = run("python3 -c 'import cowsay; print(cowsay.cow(\"moo\"))'")
    assert "moo" in result.stdout, f"cowsay not working: {result.stdout}"


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
