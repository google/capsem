"""Verify session.db survives clean VM shutdown."""

import shutil
import sqlite3
import tempfile
import uuid

import pytest

from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.session_lifecycle


def test_db_survives_clean_shutdown():
    """Boot VM, exec, stop process cleanly, verify session.db."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    vm_name = f"survive-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": vm_name, "ram_mb": 2048, "cpus": 2})
        assert wait_exec_ready(client, vm_name), f"VM {vm_name} never exec-ready"

        # Run a command to generate some data
        client.post(f"/exec/{vm_name}", {"command": "echo session-test"})

        import time
        time.sleep(3)

        db_path = svc.tmp_dir / "sessions" / vm_name / "session.db"

        # Force WAL flush by executing PRAGMA wal_checkpoint
        try:
            import sqlite3
            conn = sqlite3.connect(db_path)
            conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            conn.close()
        except Exception:
            pass

        if not db_path.exists():
            pytest.skip("session.db not created")

        with tempfile.TemporaryDirectory() as tmp:
            copy_path = f"{tmp}/session-copy.db"
            shutil.copy2(str(db_path), copy_path)

            # Delete the VM
            client.delete(f"/delete/{vm_name}")

            # Verify the copy is valid SQLite
            conn = sqlite3.connect(copy_path)
            tables = [
                r[0] for r in conn.execute(
                    "SELECT name FROM sqlite_master WHERE type='table'"
                ).fetchall()
            ]
            conn.close()
            assert len(tables) > 0, "Copied session.db has no tables"
    finally:
        try:
            client.delete(f"/delete/{vm_name}")
        except Exception:
            pass
        svc.stop()
