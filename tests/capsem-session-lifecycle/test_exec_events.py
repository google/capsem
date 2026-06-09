"""Verify exec commands generate net_events in session.db."""

import time

import pytest

pytestmark = pytest.mark.session_lifecycle


def test_exec_curl_creates_net_event(lifecycle_env, lifecycle_db, lifecycle_debug_upstream):
    """An HTTPS request from guest should appear in net_events."""
    client, vm_name, _, _ = lifecycle_env

    # Trigger deterministic local HTTP telemetry without relying on public DNS
    # or Internet reachability.
    client.post(f"/vms/{vm_name}/exec", {
        "command": f"curl -s -o /dev/null --max-time 5 {lifecycle_debug_upstream}/tiny || true"
    })

    # Wait for async writer to flush
    time.sleep(3)

    rows = lifecycle_db.execute(
        "SELECT domain, decision FROM net_events WHERE domain = '127.0.0.1'"
    ).fetchall()
    # Should have at least one event for the curl request
    assert len(rows) > 0, "Expected at least one net_event from curl request"
