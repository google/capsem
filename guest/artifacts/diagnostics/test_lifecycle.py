"""VM lifecycle diagnostics -- sysutil suspend, identity, and hostname."""

import os

import pytest

from conftest import run


# -- Lifecycle binary symlinks --


REMOVED_SHUTDOWN_LINKS = [
    "/sbin/shutdown",
    "/sbin/halt",
    "/sbin/poweroff",
    "/sbin/reboot",
]


@pytest.mark.parametrize("link", REMOVED_SHUTDOWN_LINKS, ids=REMOVED_SHUTDOWN_LINKS)
def test_shutdown_symlinks_are_not_installed(link):
    """Guest shutdown commands must not be wired to capsem-sysutil."""
    assert not os.path.lexists(link), f"{link} should not be installed"


def test_suspend_symlink_exists():
    """The suspend symlink must point to capsem-sysutil."""
    link = "/usr/local/bin/suspend"
    target = "/run/capsem-sysutil"
    assert os.path.islink(link), f"{link} is not a symlink"
    actual = os.readlink(link)
    assert actual == target, f"{link} -> {actual}, expected {target}"


def test_capsem_sysutil_binary_exists():
    """capsem-sysutil must be deployed to /run/."""
    assert os.path.isfile("/run/capsem-sysutil"), "/run/capsem-sysutil not found"


def test_capsem_sysutil_not_writable():
    """capsem-sysutil must be read-only (chmod 555)."""
    import stat
    mode = os.stat("/run/capsem-sysutil").st_mode
    writable = mode & (stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH)
    assert writable == 0, f"/run/capsem-sysutil has write bits set (mode={oct(mode)})"


def test_shutdown_command_is_disabled():
    """Direct capsem-sysutil shutdown must fail instead of stopping the VM."""
    result = run("/run/capsem-sysutil shutdown")
    assert result.returncode != 0, "capsem-sysutil shutdown should fail"
    assert "disabled" in result.stderr.lower(), result.stderr


# -- VM identity --


def test_capsem_vm_id_set():
    """CAPSEM_VM_ID must be set in the guest environment."""
    vm_id = os.environ.get("CAPSEM_VM_ID")
    assert vm_id, "CAPSEM_VM_ID env var is not set"
    assert len(vm_id) > 0, "CAPSEM_VM_ID is empty"


def test_capsem_vm_name_set():
    """CAPSEM_VM_NAME must be set in the guest environment."""
    vm_name = os.environ.get("CAPSEM_VM_NAME")
    assert vm_name, "CAPSEM_VM_NAME env var is not set"
    assert len(vm_name) > 0, "CAPSEM_VM_NAME is empty"


def test_hostname_is_not_default():
    """Hostname must be set to CAPSEM_VM_NAME (not 'localhost' or empty)."""
    result = run("hostname")
    assert result.returncode == 0
    hostname = result.stdout.strip()
    assert hostname not in ("", "localhost", "(none)", "capsem"), \
        f"hostname is default/empty: {hostname!r}"
    # Should match CAPSEM_VM_NAME if set
    expected = os.environ.get("CAPSEM_VM_NAME")
    if expected:
        assert hostname == expected, \
            f"hostname {hostname!r} != CAPSEM_VM_NAME {expected!r}"


def test_hostname_matches_vm_id_or_name():
    """Hostname must match either CAPSEM_VM_NAME or CAPSEM_VM_ID."""
    result = run("hostname")
    assert result.returncode == 0
    hostname = result.stdout.strip()
    vm_name = os.environ.get("CAPSEM_VM_NAME", "")
    vm_id = os.environ.get("CAPSEM_VM_ID", "")
    assert hostname in (vm_name, vm_id), \
        f"hostname {hostname!r} doesn't match VM_NAME={vm_name!r} or VM_ID={vm_id!r}"
