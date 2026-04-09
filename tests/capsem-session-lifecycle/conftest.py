"""Shared fixtures for session.db lifecycle tests."""

import sqlite3
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.session_lifecycle


@pytest.fixture(scope="session")
def lifecycle_env():
    """Start service, boot VM, wait for exec-ready. Returns (client, vm_name, tmp_dir)."""
    svc = ServiceInstance()
    svc.start()

    client = svc.client()
    vm_name = f"lifecycle-{uuid.uuid4().hex[:8]}"
    client.post("/provision", {"name": vm_name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})

    if not wait_exec_ready(client, vm_name):
        svc.stop()
        pytest.fail(f"VM {vm_name} never became exec-ready")

    yield client, vm_name, svc.tmp_dir, svc

    try:
        client.delete(f"/delete/{vm_name}")
    except Exception:
        pass
    svc.stop()


@pytest.fixture
def lifecycle_db(lifecycle_env):
    """Open the VM's session.db as read-only sqlite3 connection."""
    _, vm_name, tmp_dir, _ = lifecycle_env
    db_path = tmp_dir / "sessions" / vm_name / "session.db"
    if not db_path.exists():
        pytest.skip(f"session.db not found at {db_path}")
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    yield conn
    conn.close()
