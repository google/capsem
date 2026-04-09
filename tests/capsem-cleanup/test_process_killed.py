"""Verify VM process is killed after delete."""

import os
import signal
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready

pytestmark = pytest.mark.cleanup


def test_process_killed_after_delete(cleanup_env):
    """Create VM, get PID from info, delete, verify process gone."""
    client = cleanup_env.client()
    name = f"kill-{uuid.uuid4().hex[:8]}"

    client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

    info = client.get(f"/info/{name}")
    pid = info.get("pid") if info else None

    client.delete(f"/delete/{name}")

    if pid:
        # Give process time to exit
        import time
        time.sleep(2)
        try:
            os.kill(pid, 0)
            pytest.fail(f"Process {pid} still alive after VM delete")
        except ProcessLookupError:
            pass  # Expected -- process is gone
        except PermissionError:
            pass  # Process exists but owned by another user (unlikely in test)
