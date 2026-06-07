"""Verify MITM proxy policy enforcement and telemetry logging."""

import os
import sqlite3
import uuid
import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.gateway


@pytest.fixture(scope="module")
def service_env():
    """Start a real capsem-service on an isolated temp socket."""
    svc = ServiceInstance()
    svc.start()
    yield svc
    svc.stop()


@pytest.fixture
def client(service_env):
    """UDS HTTP client connected to the test service."""
    return service_env.client()


def test_mitm_policy_telemetry(service_env, client):
    """Blocked domain access attempts are logged in session DB."""
    vm_name = f"mitm-telemetry-{uuid.uuid4().hex[:8]}"
    
    # Provision VM
    client.post("/provision", {"name": vm_name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    
    try:
        assert wait_exec_ready(client, vm_name, timeout=EXEC_READY_TIMEOUT)
        
        # Try to access a domain that should be blocked by default policy
        blocked_domain = "malware.example.com"
        
        # Run curl in guest
        client.post(f"/exec/{vm_name}", {
            "command": f"curl -s https://{blocked_domain} || true"
        })
        
        # Wait a bit for telemetry to be flushed to DB
        import time
        time.sleep(2)
        
        # Check session.db
        # ServiceInstance creates a temp dir, and sessions are in `sessions/` subdirectory
        db_path = service_env.tmp_dir / "sessions" / vm_name / "session.db"
        
        assert db_path.exists(), f"Session DB not found at {db_path}"
        
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        try:
            cursor = conn.execute(
                "SELECT qname, decision, rcode FROM dns_events WHERE qname = ?",
                (blocked_domain,),
            )
            row = cursor.fetchone()
            assert row is not None, f"No dns_event found for {blocked_domain}"
            assert row[1] == "denied", f"Expected denied DNS decision, got: {row[1]}"
            assert row[2] == 3, f"Expected NXDOMAIN rcode=3, got: {row[2]}"

            cursor = conn.execute(
                "SELECT COUNT(*) FROM net_events WHERE domain = ? AND decision = 'allowed'",
                (blocked_domain,),
            )
            allowed_count = cursor.fetchone()[0]
            assert allowed_count == 0, (
                f"Domain {blocked_domain} should not have allowed net_events"
            )
        finally:
            conn.close()
            
    finally:
        try:
            client.delete(f"/vms/{vm_name}/delete")
        except Exception:
            pass
