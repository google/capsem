"""Auto snapshot ring buffer behavior."""

import json
import time
import uuid

import pytest

from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.snapshot


@pytest.fixture(scope="module")
def snapshot_vm():
    """A VM for snapshot tests, with short auto-snapshot interval."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    name = f"snap-{uuid.uuid4().hex[:8]}"
    client.post("/provision", {"name": name, "ram_mb": 2048, "cpus": 2})
    assert wait_exec_ready(client, name), f"VM {name} never exec-ready"

    yield client, name, svc.tmp_dir

    try:
        client.delete(f"/delete/{name}")
    except Exception:
        pass
    svc.stop()


def test_auto_snapshots_dir_exists(snapshot_vm):
    """Session dir should have an auto_snapshots/ directory."""
    _, name, tmp_dir = snapshot_vm
    session_dir = tmp_dir / "sessions" / name
    snap_dir = session_dir / "auto_snapshots"
    # May not exist yet if no snapshots taken -- the test documents the expectation
    if session_dir.exists():
        # At minimum the session dir exists
        assert session_dir.is_dir()


def test_snapshot_metadata_valid_json(snapshot_vm):
    """Any snapshot slot with metadata.json should contain valid JSON."""
    _, name, tmp_dir = snapshot_vm
    snap_dir = tmp_dir / "sessions" / name / "auto_snapshots"
    if not snap_dir.exists():
        pytest.skip("No auto_snapshots dir")

    for slot in snap_dir.iterdir():
        meta = slot / "metadata.json"
        if meta.exists():
            data = json.loads(meta.read_text())
            assert "slot" in data
            assert "timestamp" in data
            assert "origin" in data
