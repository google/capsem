"""Verify exec commands generate net_events in session.db."""

import time

import pytest

pytestmark = pytest.mark.session_lifecycle


def test_exec_curl_creates_net_event(lifecycle_env, lifecycle_db):
    """An HTTPS request from guest should appear in net_events."""
    client, vm_name, _, _ = lifecycle_env

    # Trigger a deterministic denied network request. This proves logging
    # without relying on any external service.
    client.post(f"/vms/{vm_name}/exec", {
        "command": "curl -skI --connect-timeout 5 https://evil-never-allowed.invalid 2>&1 || true"
    })

    # Wait for async writer to flush
    time.sleep(3)

    rows = lifecycle_db.execute(
        "SELECT domain, decision FROM net_events"
    ).fetchall()
    # Should have at least one event for the curl request
    assert len(rows) > 0, "Expected at least one net_event from curl request"
