"""Rapid sequential exec commands on a single VM."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.stress


def test_rapid_exec_sequence():
    """Run 20 execs in rapid succession on one VM -- all should complete."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"rapid-exec-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        results = []
        for i in range(20):
            resp = client.post(f"/exec/{name}", {"command": f"echo seq-{i}"})
            results.append(resp)

        # All should have returned
        for i, resp in enumerate(results):
            assert resp is not None, f"Exec {i} returned None"
            assert f"seq-{i}" in resp.get("stdout", ""), f"Exec {i} missing output"

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
        svc.stop()


def test_rapid_file_io():
    """Write and read 10 files in rapid succession."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"rapid-io-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        # Write 10 files
        for i in range(10):
            resp = client.post(f"/write_file/{name}", {
                "path": f"/root/file-{i}.txt",
                "content": f"content-{i}",
            })
            assert resp is not None, f"Write {i} failed"

        # Read them all back
        for i in range(10):
            resp = client.post(f"/read_file/{name}", {"path": f"/root/file-{i}.txt"})
            assert resp is not None, f"Read {i} failed"
            assert f"content-{i}" in resp.get("content", ""), f"File {i} content mismatch"

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
        svc.stop()
