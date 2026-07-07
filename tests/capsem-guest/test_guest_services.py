"""Verify expected guest services are running after boot."""

import pytest

pytestmark = pytest.mark.guest


class TestGuestServices:

    def test_pty_agent_running(self, guest_env):
        """capsem-pty-agent process is running in guest."""
        client, name = guest_env
        resp = client.post(
            f"/vms/{name}/exec",
            {"command": "ps -eo args= | grep '^/run/capsem-pty-agent$' || true"},
        )
        assert resp is not None
        stdout = resp.get("stdout", "").strip()
        assert len(stdout) > 0, "capsem-pty-agent not found running"

    def test_net_proxy_running(self, guest_env):
        """capsem-net-proxy process is running in guest."""
        client, name = guest_env
        resp = client.post(
            f"/vms/{name}/exec",
            {"command": "ps -eo args= | grep '^/run/capsem-net-proxy$' || true"},
        )
        assert resp is not None
        stdout = resp.get("stdout", "").strip()
        assert len(stdout) > 0, "capsem-net-proxy not found running"

    def test_dns_proxy_running(self, guest_env):
        """capsem-dns-proxy DNS resolver is running in guest."""
        client, name = guest_env
        resp = client.post(
            f"/vms/{name}/exec",
            {"command": "ps -eo args= | grep '^/run/capsem-dns-proxy$' || true"},
        )
        assert resp is not None
        stdout = resp.get("stdout", "").strip()
        assert len(stdout) > 0, "capsem-dns-proxy not found running"
