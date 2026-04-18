"""POST /fork/{id}: clone a persistent VM's state into a new persistent VM."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready, vm_name

pytestmark = pytest.mark.integration


def _provision_persistent(client, prefix="fork"):
    """Provision a persistent (named) VM and return its name."""
    name = vm_name(prefix)
    resp = client.post("/provision", {
        "name": name,
        "ram_mb": DEFAULT_RAM_MB,
        "cpus": DEFAULT_CPUS,
        "persistent": True,
    })
    assert resp is not None and resp.get("id") == name, f"provision failed: {resp}"
    return name


class TestFork:

    def test_fork_running_persistent(self, client):
        """Forking a running persistent VM preserves workspace content in the child."""
        source = _provision_persistent(client, "fork-src")
        children = []
        try:
            assert wait_exec_ready(client, source, timeout=EXEC_READY_TIMEOUT), (
                f"source {source} never exec-ready"
            )

            marker = f"fork-marker-{uuid.uuid4().hex[:8]}"
            client.post(f"/write_file/{source}", {
                "path": "/root/fork-marker.txt",
                "content": marker,
            })

            child = f"fork-child-{uuid.uuid4().hex[:6]}"
            children.append(child)
            resp = client.post(f"/fork/{source}", {
                "name": child,
                "description": "coverage test fork",
            }, timeout=60)
            assert resp is not None
            assert resp.get("name") == child, f"unexpected fork response: {resp}"
            assert resp.get("size_bytes", 0) > 0, f"fork size 0: {resp}"

            # Child is registered persistent/stopped. Resume to read the marker.
            resume_resp = client.post(f"/resume/{child}", {})
            assert resume_resp is not None, f"resume failed: {resume_resp}"
            resumed_id = resume_resp.get("id", child)
            assert wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT), (
                f"forked VM {resumed_id} did not become exec-ready"
            )
            read = client.post(f"/read_file/{resumed_id}", {"path": "/root/fork-marker.txt"})
            assert read is not None
            assert read.get("content") == marker, (
                f"marker did not survive fork: {read}"
            )
        finally:
            for vm in children + [source]:
                try:
                    client.delete(f"/delete/{vm}")
                except Exception:
                    pass

    def test_fork_duplicate_name_rejected(self, client):
        """Fork into a name that is already a registered persistent VM fails."""
        source = _provision_persistent(client, "fork-dup-src")
        taken = _provision_persistent(client, "fork-dup-dest")
        try:
            resp = client.post(f"/fork/{source}", {"name": taken}, timeout=30)
            assert resp is not None
            assert "error" in resp or "already exists" in str(resp).lower(), (
                f"expected duplicate name rejection, got: {resp}"
            )
        finally:
            for vm in (source, taken):
                try:
                    client.delete(f"/delete/{vm}")
                except Exception:
                    pass

    def test_fork_nonexistent_source(self, client):
        """Fork from an unknown source id fails with 404."""
        resp = client.post(
            f"/fork/ghost-{uuid.uuid4().hex[:6]}",
            {"name": f"child-{uuid.uuid4().hex[:6]}"},
            timeout=15,
        )
        assert resp is not None
        assert "error" in resp or "not found" in str(resp).lower(), (
            f"expected 404 for missing source, got: {resp}"
        )
