"""Shared fixtures for session.db telemetry tests."""

import sqlite3
import time
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.session


@pytest.fixture(scope="session")
def session_env():
    """Start service, boot a VM, wait for exec-ready. Returns (client, vm_name, tmp_dir)."""
    svc = ServiceInstance()
    svc.start()

    client = svc.client()
    vm_name = f"sess-{uuid.uuid4().hex[:8]}"
    client.post("/provision", {"name": vm_name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})

    if not wait_exec_ready(client, vm_name):
        svc.stop()
        pytest.fail(f"VM {vm_name} never became exec-ready")

    yield client, vm_name, svc.tmp_dir

    try:
        client.delete(f"/delete/{vm_name}")
    except Exception:
        pass
    svc.stop()


@pytest.fixture
def session_db(session_env):
    """Open the VM's session.db as a read-only sqlite3 connection."""
    _, vm_name, tmp_dir = session_env
    db_path = tmp_dir / "sessions" / vm_name / "session.db"
    if not db_path.exists():
        pytest.skip(f"session.db not found at {db_path}")
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    yield conn
    conn.close()
