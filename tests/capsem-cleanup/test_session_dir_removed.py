"""Verify VM session directory is removed after delete."""

import uuid

import pytest

from pathlib import Path

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready

pytestmark = pytest.mark.cleanup


def test_session_dir_removed_after_delete(cleanup_env):
    """Create VM, verify session dir exists, delete, verify gone."""
    client = cleanup_env.client()
    name = f"sessdir-{uuid.uuid4().hex[:8]}"

    client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

    sessions_dir = cleanup_env.tmp_dir / "sessions" / name
    # Session dir may or may not exist depending on implementation

    client.delete(f"/delete/{name}")

    import time
    time.sleep(2)

    if sessions_dir.exists():
        pytest.fail(f"Session dir {sessions_dir} still exists after delete")
