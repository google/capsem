"""Verify WAL file is cleaned up after clean shutdown."""

import sqlite3
import uuid

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, vm_session_db_path, wait_exec_ready

pytestmark = pytest.mark.session_lifecycle


def test_wal_absent_after_clean_shutdown():
    """After clean VM shutdown, session.db WAL file should be absent or empty."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"wal-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        # Generate some activity to create WAL entries
        client.post(f"/vms/{name}/exec", {"command": "echo wal-test"})

        db_path = vm_session_db_path(svc.tmp_dir, client, name)

        # Stop is the clean-shutdown operation for a retained session. DELETE
        # permanently destroys the database and cannot prove its final WAL
        # state after returning.
        client.post(f"/vms/{name}/stop", {})

        # Check WAL state
        wal_path = db_path.with_suffix(".db-wal")

        if wal_path.exists():
            # WAL may exist but should be empty (checkpointed)
            wal_size = wal_path.stat().st_size
            assert wal_size == 0, \
                f"WAL file should be empty after clean shutdown, got {wal_size} bytes"

        # DB must still be readable after stop. This is intentionally not
        # conditional: a missing database is a clean-shutdown failure.
        assert db_path.exists(), f"session DB disappeared after clean stop: {db_path}"
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        tables = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
        conn.close()
        assert len(tables) > 0, "DB should have tables"

    finally:
        svc.stop()
