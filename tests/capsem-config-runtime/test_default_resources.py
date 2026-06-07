"""Verify default CPU and RAM values are applied in guest."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready

pytestmark = pytest.mark.config_runtime


def test_default_cpu_count(config_svc):
    """VM provisioned with default cpus reports correct nproc."""
    client = config_svc.client()
    name = f"defcpu-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": 4})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/exec/{name}", {"command": "nproc"})
        nproc = int(resp.get("stdout", "0").strip()) if resp else 0
        assert nproc == 4, f"Expected 4 CPUs, got {nproc}"
    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass


def test_default_ram(config_svc):
    """VM provisioned with 4096MB reports ~4096MB of memory."""
    client = config_svc.client()
    name = f"defram-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": 4096, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/exec/{name}", {"command": "free -m | awk '/Mem:/ {print $2}'"})
        total_mb = int(resp.get("stdout", "0").strip()) if resp else 0
        # Allow 10% tolerance for kernel overhead
        assert total_mb > 3600, f"Expected ~4096MB, got {total_mb}MB"
    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
