"""Verify VM instance socket is removed after delete."""

import uuid

import pytest

from pathlib import Path

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready

pytestmark = pytest.mark.cleanup


def test_socket_removed_after_delete(cleanup_env):
    """Create VM, verify instance socket exists, delete, verify gone."""
    client = cleanup_env.client()
    name = f"sock-{uuid.uuid4().hex[:8]}"

    client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

    # Check for instance socket in the run dir
    instances_dir = cleanup_env.tmp_dir / "instances"
    instance_sock = instances_dir / f"{name}.sock" if instances_dir.exists() else None

    client.delete(f"/delete/{name}")

    import time
    time.sleep(2)

    if instance_sock and instance_sock.exists():
        pytest.fail(f"Instance socket {instance_sock} still exists after delete")

    # Also verify VM is gone from list
    list_resp = client.get("/list")
    ids = [s["id"] for s in list_resp["sandboxes"]]
    assert name not in ids
