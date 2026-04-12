"""Verify WAL file is cleaned up after clean shutdown."""

import sqlite3
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.session_lifecycle


def test_wal_absent_after_clean_shutdown():
    """After clean VM shutdown, session.db WAL file should be absent or empty."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"wal-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        # Generate some activity to create WAL entries
        client.post(f"/exec/{name}", {"command": "echo wal-test"})

        # Clean shutdown
        client.delete(f"/delete/{name}")

        # Check WAL state
        db_path = svc.tmp_dir / "sessions" / name / "session.db"
        wal_path = db_path.with_suffix(".db-wal")

        if wal_path.exists():
            # WAL may exist but should be empty (checkpointed)
            wal_size = wal_path.stat().st_size
            assert wal_size == 0, \
                f"WAL file should be empty after clean shutdown, got {wal_size} bytes"

        # DB should still be readable
        if db_path.exists():
            conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
            tables = conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            ).fetchall()
            conn.close()
            assert len(tables) > 0, "DB should have tables"

    finally:
        svc.stop()
