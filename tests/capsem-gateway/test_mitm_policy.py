"""Verify MITM proxy policy enforcement and telemetry logging."""

import os
import json
import selectors
import sqlite3
import subprocess
import time
import uuid
from pathlib import Path

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = pytest.mark.gateway

PROJECT_ROOT = Path(__file__).parent.parent.parent
DEBUG_UPSTREAM_BINARY = PROJECT_ROOT / "target" / "debug" / "capsem-debug-upstream"
DEBUG_UPSTREAM_ADDR = "127.0.0.1:3713"


def _read_ready_json(proc, timeout_s=10):
    selector = selectors.DefaultSelector()
    selector.register(proc.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout_s
    lines = []
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(
                f"capsem-debug-upstream exited early with code {proc.returncode}: "
                f"{''.join(lines)}"
            )
        for key, _ in selector.select(timeout=0.2):
            line = key.fileobj.readline()
            if not line:
                continue
            lines.append(line)
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            if payload.get("service") == "capsem-debug-upstream":
                return payload
    raise TimeoutError(
        "capsem-debug-upstream did not print ready JSON; "
        f"stdout={''.join(lines)!r}"
    )


def _stop_process(proc):
    if proc is None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)
    if proc.stdout is not None:
        proc.stdout.close()


@pytest.fixture(scope="module")
def debug_upstream():
    if not DEBUG_UPSTREAM_BINARY.exists():
        pytest.skip(
            f"{DEBUG_UPSTREAM_BINARY} not found; run `cargo build -p capsem-debug-upstream`"
        )
    proc = subprocess.Popen(
        [str(DEBUG_UPSTREAM_BINARY), "--addr", DEBUG_UPSTREAM_ADDR],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    try:
        ready = _read_ready_json(proc)
        yield ready["base_url"]
    finally:
        _stop_process(proc)


@pytest.fixture(scope="module")
def service_env(debug_upstream):
    """Start a real capsem-service on an isolated temp socket."""
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    os.environ["CAPSEM_CORP_CONFIG"] = str(PROJECT_ROOT / "config" / "integration-test-corp.toml")
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
            "command": f"curl -s -o /dev/null -w '%{{http_code}}' --max-time 5 http://{DEBUG_UPSTREAM_ADDR}/deny-target || true"
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
