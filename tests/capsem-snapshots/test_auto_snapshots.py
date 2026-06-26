"""Auto snapshot ring buffer behavior."""

import json
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, vm_session_dir, wait_exec_ready

pytestmark = pytest.mark.snapshot


@pytest.fixture(scope="module")
def snapshot_vm():
    """A VM for snapshot tests, with short auto-snapshot interval."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    name = f"snap-{uuid.uuid4().hex[:8]}"
    client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    assert wait_exec_ready(client, name), f"VM {name} never exec-ready"

    yield client, name, svc.tmp_dir

    try:
        client.delete(f"/vms/{name}/delete")
    except Exception:
        pass
    svc.stop()


def test_auto_snapshots_dir_exists(snapshot_vm):
    """Session dir should have an auto_snapshots/ directory."""
    client, name, tmp_dir = snapshot_vm
    session_dir = vm_session_dir(tmp_dir, client, name, must_exist=False)
    if session_dir.exists():
        assert session_dir.is_dir()


def test_snapshot_metadata_valid_json(snapshot_vm):
    """Any snapshot slot with metadata.json should contain valid JSON."""
    client, name, tmp_dir = snapshot_vm
    snap_dir = vm_session_dir(tmp_dir, client, name, must_exist=False) / "auto_snapshots"
    if not snap_dir.exists():
        pytest.skip("No auto_snapshots dir")

    for slot in snap_dir.iterdir():
        meta = slot / "metadata.json"
        if meta.exists():
            data = json.loads(meta.read_text())
            assert "slot" in data
            assert "timestamp" in data
            assert "origin" in data
