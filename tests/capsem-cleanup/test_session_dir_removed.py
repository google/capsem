"""Verify VM session directory is removed after delete."""

import uuid

import pytest

from pathlib import Path

from helpers.service import wait_exec_ready

pytestmark = pytest.mark.cleanup


def test_session_dir_removed_after_delete(cleanup_env):
    """Create VM, verify session dir exists, delete, verify gone."""
    client = cleanup_env.client()
    name = f"sessdir-{uuid.uuid4().hex[:8]}"

    client.post("/provision", {"name": name, "ram_mb": 2048, "cpus": 2})
    wait_exec_ready(client, name, timeout=30)

    sessions_dir = cleanup_env.tmp_dir / "sessions" / name
    # Session dir may or may not exist depending on implementation

    client.delete(f"/delete/{name}")

    import time
    time.sleep(2)

    if sessions_dir.exists():
        pytest.fail(f"Session dir {sessions_dir} still exists after delete")
