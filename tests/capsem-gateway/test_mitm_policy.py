"""Verify MITM proxy policy enforcement and telemetry logging."""

import os
import sqlite3
import time
import uuid
from pathlib import Path

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.mock_server import MOCK_SERVER_BINARY, MOCK_SERVER_ADDR, start_mock_server, stop_process
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.gateway

PROJECT_ROOT = Path(__file__).parent.parent.parent


@pytest.fixture(scope="module")
def mock_server():
    if not MOCK_SERVER_BINARY.exists():
        pytest.fail(f"{MOCK_SERVER_BINARY} not found; restore scripts/mock_server_runtime.py")
    proc, ready = start_mock_server()
    try:
        yield ready["base_url"]
    finally:
        stop_process(proc)


@pytest.fixture(scope="module")
def service_env(mock_server):
    """Start a real capsem-service on an isolated temp socket."""
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    os.environ["CAPSEM_CORP_CONFIG"] = str(
        PROJECT_ROOT / "tests" / "fixtures" / "config" / "integration" / "corp.toml"
    )
    svc = ServiceInstance()
    svc.start()
    try:
        yield svc
    finally:
        svc.stop()
        if old_corp_config is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp_config


@pytest.fixture
def client(service_env):
    """UDS HTTP client connected to the test service."""
    return service_env.client()


def test_mitm_policy_telemetry(service_env, client):
    """Blocked domain access attempts are logged in session DB."""
    vm_name = f"mitm-telemetry-{uuid.uuid4().hex[:8]}"
    
    # Provision VM
    client.post(
        "/vms/create",
        {
            "name": vm_name,
            "profile_id": CODE_PROFILE_ID,
            "ram_mb": DEFAULT_RAM_MB,
            "cpus": DEFAULT_CPUS,
        },
    )
    
    try:
        assert wait_exec_ready(client, vm_name, timeout=EXEC_READY_TIMEOUT)
        
        # The corp integration rule blocks the deterministic local debug
        # upstream path. This proves the single CEL/security-event rail without
        # resurrecting the retired default-domain block path.
        client.post(f"/vms/{vm_name}/exec", {
            "command": f"curl -s -o /dev/null -w '%{{http_code}}' --max-time 5 http://{MOCK_SERVER_ADDR}/deny-target || true"
        })

        # Wait a bit for telemetry to be flushed to DB
        time.sleep(2)
        
        # Check session.db
        # ServiceInstance creates a temp dir, and sessions are in `sessions/` subdirectory
        db_path = service_env.tmp_dir / "sessions" / vm_name / "session.db"
        
        assert db_path.exists(), f"Session DB not found at {db_path}"
        
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        try:
            cursor = conn.execute(
                """
                SELECT domain, path, decision, policy_rule
                FROM net_events
                WHERE domain = '127.0.0.1' AND path = '/deny-target'
                ORDER BY id DESC
                LIMIT 1
                """,
            )
            row = cursor.fetchone()
            assert row is not None, "No net_event found for local /deny-target"
            assert row[2] == "denied", f"Expected denied decision, got: {row[2]}"
            assert row[3] == "corp.rules.block_local_deny_target"

            cursor = conn.execute(
                """
                SELECT COUNT(*)
                FROM net_events
                WHERE domain = '127.0.0.1'
                  AND path = '/deny-target'
                  AND decision = 'allowed'
                """,
            )
            allowed_count = cursor.fetchone()[0]
            assert allowed_count == 0, (
                "local /deny-target should not have allowed net_events"
            )
        finally:
            conn.close()
            
    finally:
        try:
            client.delete(f"/vms/{vm_name}/delete")
        except Exception:
            pass
