"""Verify custom CPU and RAM values are applied in guest."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready

pytestmark = pytest.mark.config_runtime


def test_custom_cpu_count(config_svc):
    """VM provisioned with cpus=2 reports 2 CPUs."""
    client = config_svc.client()
    name = f"custcpu-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/exec/{name}", {"command": "nproc"})
        nproc = int(resp.get("stdout", "0").strip()) if resp else 0
        assert nproc == 2, f"Expected 2 CPUs, got {nproc}"
    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass


def test_custom_ram(config_svc):
    """VM provisioned with ram_mb=2048 reports ~2048MB."""
    client = config_svc.client()
    name = f"custram-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/exec/{name}", {"command": "free -m | awk '/Mem:/ {print $2}'"})
        total_mb = int(resp.get("stdout", "0").strip()) if resp else 0
        assert total_mb > 1800, f"Expected ~2048MB, got {total_mb}MB"
        assert total_mb < 2500, f"Got {total_mb}MB, expected ~2048MB"
    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
