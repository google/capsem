"""Verify file writes become durable fs_events after clean VM shutdown."""

import contextlib
import sqlite3
import uuid

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import vm_session_db_path, wait_exec_ready

pytestmark = pytest.mark.session_lifecycle


def test_file_write_creates_durable_fs_event(lifecycle_env):
    """A successful API write must survive the DB-owned shutdown barrier."""
    client, _, tmp_dir, _ = lifecycle_env
    name = f"file-ledger-{uuid.uuid4().hex[:8]}"
    vm_id = None
    try:
        created = client.post(
            "/vms/create",
            {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
        )
        vm_id = created["id"]
        assert wait_exec_ready(client, vm_id), f"VM {vm_id} never exec-ready"

        filename = f"file-ledger-{uuid.uuid4().hex[:8]}.txt"
        content = "durable file-event proof"
        response = client.post(
            f"/vms/{vm_id}/files/write",
            {"path": f"/root/{filename}", "content": content},
        )
        assert response == {"success": True}
        db_path = vm_session_db_path(tmp_dir, client, vm_id)

        client.post(f"/vms/{vm_id}/stop", {})

        assert db_path.exists(), f"session DB disappeared after clean stop: {db_path}"
        with contextlib.closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as db:
            rows = db.execute(
                "SELECT event_id, action, path, size FROM fs_events "
                "WHERE action = 'created' AND path = ? ORDER BY id",
                (filename,),
            ).fetchall()
        assert rows, f"missing durable created event for {filename}"
        assert all(row[0] and len(row[0]) == 12 for row in rows)
        assert all(row[1:] == ("created", filename, len(content.encode())) for row in rows)
    finally:
        if vm_id is not None:
            with contextlib.suppress(Exception):
                client.delete(f"/vms/{vm_id}/delete")
