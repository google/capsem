"""Verify that guest cannot escape VirtioFS sandbox via path traversal."""

import uuid
import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import wait_exec_ready

pytestmark = pytest.mark.security


def test_virtiofs_path_traversal(client):
    """Attempt to read host file outside workspace via path traversal."""
    vm_name = f"traversal-{uuid.uuid4().hex[:8]}"
    
    # Provision VM
    resp = client.post("/provision", {"name": vm_name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    assert resp is not None
    
    try:
        assert wait_exec_ready(client, vm_name), f"VM {vm_name} never became exec-ready"
        
        # The VirtioFS share is `session_dir/guest/`.
        # Host-only files like `session.db` are in `session_dir/`.
        # So `../session.db` relative to the mount point root should reach it if escape is possible.
        
        # We assume the mount point is `/root` based on `test_workspace_exists`.
        # We try to read `/root/../session.db`.
        # If it escapes, it will read the host's session.db.
        # If it doesn't escape, it resolves to `/session.db` in the guest, which should not exist.
        
        traversal_path = "/root/../session.db"
        
        resp = client.post(f"/exec/{vm_name}", {"command": f"cat {traversal_path} 2>&1"})
        stdout = resp.get("stdout", "") if resp else ""
        
        # If it leaked, we might see SQLite header or content.
        # If it failed safely, it should be "No such file or directory" or similar.
        
        # SQLite files start with "SQLite format 3"
        assert "SQLite format 3" not in stdout, f"Host session.db leaked! Output: {stdout}"
        
        # Also verify that it didn't find it (expected behavior if safe)
        assert "No such file or directory" in stdout, f"Expected 'No such file or directory', got: {stdout}"
        
    finally:
        # Cleanup
        try:
            client.delete(f"/delete/{vm_name}")
        except Exception:
            pass
