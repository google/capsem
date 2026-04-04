"""Verify blocked domains are enforced at runtime."""

import uuid

import pytest

from helpers.service import wait_exec_ready

pytestmark = pytest.mark.config_runtime


def test_blocked_domain_denied(config_svc):
    """curl to a blocked domain should fail or be denied by proxy."""
    client = config_svc.client()
    name = f"block-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": 2048, "cpus": 2})
        assert wait_exec_ready(client, name, timeout=30)

        # Try to access a domain that should be blocked by default policy
        # Most policies block everything except an allowlist
        resp = client.post(f"/exec/{name}", {
            "command": "curl -s -o /dev/null -w '%{http_code}' --max-time 5 https://malware.example.com 2>&1; echo exit=$?"
        })
        stdout = resp.get("stdout", "") if resp else ""
        # Should either fail (exit!=0), get blocked (403/502), or connection refused
        assert (
            "exit=0" not in stdout
            or "403" in stdout
            or "502" in stdout
            or "000" in stdout  # curl couldn't connect
        ), f"Blocked domain should fail, got: {stdout}"

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
