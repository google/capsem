"""Verify guest filesystem configuration at runtime."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready

pytestmark = pytest.mark.config_runtime


def test_workspace_writable(config_svc):
    """Guest workspace directory is writable."""
    client = config_svc.client()
    name = f"ws-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/exec/{name}", {
            "command": "echo test_data > /root/write_test.txt && cat /root/write_test.txt"
        })
        stdout = resp.get("stdout", "") if resp else ""
        assert "test_data" in stdout, f"Workspace not writable: {stdout}"

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass


