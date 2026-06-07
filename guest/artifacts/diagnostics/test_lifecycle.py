"""VM lifecycle diagnostics -- sysutil symlinks, identity, and hostname."""

import os

import pytest

from conftest import run


# -- Lifecycle binary symlinks --


SYSUTIL_SYMLINKS = [
    ("/sbin/shutdown", "/run/capsem-sysutil"),
    ("/sbin/halt", "/run/capsem-sysutil"),
    ("/sbin/poweroff", "/run/capsem-sysutil"),
    ("/sbin/reboot", "/run/capsem-sysutil"),
    ("/usr/local/bin/suspend", "/run/capsem-sysutil"),
]


@pytest.mark.parametrize("link,target", SYSUTIL_SYMLINKS,
                         ids=[p for p, _ in SYSUTIL_SYMLINKS])
def test_sysutil_symlink_exists(link, target):
    """Lifecycle symlinks must point to capsem-sysutil."""
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


def test_shutdown_help():
    """shutdown --help should print capsem help text."""
    result = run("shutdown --help")
    assert result.returncode == 0, f"shutdown --help failed: {result.stderr}"
    assert "capsem" in result.stdout.lower() or "sandbox" in result.stdout.lower(), \
        f"shutdown --help output doesn't mention capsem: {result.stdout}"


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
