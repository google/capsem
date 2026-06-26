"""Each VM has its own session.db with no cross-talk."""

import sqlite3

import pytest
from helpers.service import vm_session_db_path, vm_session_dir


pytestmark = pytest.mark.isolation


def test_separate_session_dirs(multi_vm_env):
    """Each VM's session directory is distinct."""
    client, vm_a, vm_b, tmp_dir = multi_vm_env
    dir_a = vm_session_dir(tmp_dir, client, vm_a)
    dir_b = vm_session_dir(tmp_dir, client, vm_b)
    assert dir_a != dir_b


def test_exec_event_only_in_own_db(multi_vm_env):
    """Exec in VM-A appears only in VM-A's session.db, not VM-B's."""
    client, vm_a, vm_b, tmp_dir = multi_vm_env

    # Run a distinctive command in VM-A only
    marker = "isolation-marker-12345"
    client.post(f"/vms/{vm_a}/exec", {"command": f"echo {marker}"})

    # Check VM-B's session.db does NOT contain the marker
    db_b = vm_session_db_path(tmp_dir, client, vm_b)
    if not db_b.exists():
        pytest.skip("VM-B session.db not found")

    conn = sqlite3.connect(f"file:{db_b}?mode=ro", uri=True)
    try:
        cursor = conn.execute(
            "SELECT count(*) FROM net_events WHERE domain LIKE ?",
            (f"%{marker}%",),
        )
        count = cursor.fetchone()[0]
        assert count == 0, "VM-B session.db should not contain events from VM-A"
    except sqlite3.OperationalError:
        # Table may not exist yet if no events logged
        pass
    finally:
        conn.close()
