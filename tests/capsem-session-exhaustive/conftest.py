"""Shared fixtures for exhaustive per-table session.db tests."""

import sqlite3
import time
import uuid

import pytest

from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.session_exhaustive


@pytest.fixture(scope="session")
def exhaustive_env():
    """Start service, boot VM, run workloads to populate session.db tables."""
    svc = ServiceInstance()
    svc.start()

    client = svc.client()
    vm_name = f"exhaust-{uuid.uuid4().hex[:8]}"
    client.post("/provision", {"name": vm_name, "ram_mb": 2048, "cpus": 2})

    if not wait_exec_ready(client, vm_name):
        svc.stop()
        pytest.fail(f"VM {vm_name} never became exec-ready")

    # Run workloads to populate tables
    # Network event: curl an allowed domain
    client.post(f"/exec/{vm_name}", {
        "command": "curl -s -o /dev/null https://elie.net/ 2>&1 || true"
    })
    # File event: write a file
    client.post(f"/write-file/{vm_name}", {
        "path": "/capsem/workspace/exhaust-test.txt",
        "content": "exhaustive test data",
    })

    # Wait for async writer to flush
    time.sleep(3)

    yield client, vm_name, svc.tmp_dir

    try:
        client.delete(f"/delete/{vm_name}")
    except Exception:
        pass
    svc.stop()


@pytest.fixture
def exhaust_db(exhaustive_env):
    """Open the VM's session.db as read-only sqlite3 connection."""
    _, vm_name, tmp_dir = exhaustive_env
    db_path = tmp_dir / "sessions" / vm_name / "session.db"
    if not db_path.exists():
        pytest.skip(f"session.db not found at {db_path}")
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    yield conn
    conn.close()
