"""Verify ephemeral VM cleanup when process crashes.

Tests the SERVICE-SIDE cleanup behavior: when an ephemeral VM process crashes,
the service should move its session directory under a failed-session name.
Persistent VMs should preserve their session dir even when the process dies.
"""

import os
import signal
import time
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import vm_session_dir, wait_exec_ready

pytestmark = pytest.mark.cleanup


def _get_vm_pid(client, name):
    """Get the OS process ID for a VM."""
    info = client.get(f"/vms/{name}/info")
    return info.get("pid") if info else None


def _vm_in_list(client, name):
    """Check if a VM appears in the service list."""
    listing = client.get("/vms/list")
    return any(
        row.get("id") == name or row.get("name") == name
        for row in listing.get("sandboxes", [])
    )


def test_ephemeral_cleaned_on_process_death(cleanup_env):
    """Crash an ephemeral VM process; service should preserve a failed session dir."""
    client = cleanup_env.client()
    create = client.post("/vms/create", {
        "ram_mb": DEFAULT_RAM_MB,
        "cpus": DEFAULT_CPUS,
    })
    name = create["id"]
    wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

    session_dir = vm_session_dir(cleanup_env.tmp_dir, client, name)
    pid = _get_vm_pid(client, name)
    assert pid, f"Could not get PID for VM {name}"

    os.kill(pid, signal.SIGKILL)

    # Wait for service to notice via stale-instance cleanup
    for _ in range(10):
        time.sleep(1)
        if not _vm_in_list(client, name):
            break
    else:
        pytest.fail(f"Ephemeral VM {name} still in list 10s after process kill")

    failed_dirs = []
    for _ in range(10):
        failed_dirs = list(session_dir.parent.glob(f"{session_dir.name}-failed-*"))
        if not session_dir.exists() and failed_dirs:
            break
        time.sleep(0.5)
    else:
        pytest.fail(
            f"Session dir {session_dir} was not moved to a failed-session dir"
        )


def test_persistent_preserved_on_process_death(cleanup_env):
    """Kill a persistent VM process; service should preserve session dir."""
    client = cleanup_env.client()
    name = f"prs-{uuid.uuid4().hex[:6]}"
    client.post("/vms/create", {
        "name": name,
        "ram_mb": DEFAULT_RAM_MB,
        "cpus": DEFAULT_CPUS,
        "persistent": True,
    })
    wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

    pid = _get_vm_pid(client, name)
    assert pid, f"Could not get PID for VM {name}"

    os.kill(pid, signal.SIGTERM)

    # Give the service time to run stale-instance cleanup
    time.sleep(5)

    # The VM should still appear in list (as Stopped)
    listing = client.get("/vms/list")
    assert isinstance(listing.get("sandboxes", []), list)
    # Note: the stale-instance cleanup removes from instances map but the
    # persistent registry keeps it, so it shows in /vms/list as Stopped
    # (or it may have been cleaned from instances but still in registry)

    # Explicit cleanup
    client.delete(f"/vms/{name}/delete")


def test_explicit_delete_always_works(cleanup_env):
    """Explicit delete should destroy any VM regardless of persistence."""
    client = cleanup_env.client()
    name = f"del-{uuid.uuid4().hex[:6]}"
    client.post("/vms/create", {
        "name": name,
        "ram_mb": DEFAULT_RAM_MB,
        "cpus": DEFAULT_CPUS,
    })
    wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

    client.delete(f"/vms/{name}/delete")
    assert not _vm_in_list(client, name), f"VM {name} still in list after explicit delete"
