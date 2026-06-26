"""Verify VM session directory is removed after delete."""

import uuid

import pytest


from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import vm_session_dir, wait_exec_ready

pytestmark = pytest.mark.cleanup


def test_session_dir_removed_after_delete(cleanup_env):
    """Create VM, verify session dir exists, delete, verify gone."""
    client = cleanup_env.client()
    name = f"sessdir-{uuid.uuid4().hex[:8]}"

    client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

    session_dir = vm_session_dir(cleanup_env.tmp_dir, client, name)

    client.delete(f"/vms/{name}/delete")

    import time
    time.sleep(2)

    if session_dir.exists():
        pytest.fail(f"Session dir {session_dir} still exists after delete")
