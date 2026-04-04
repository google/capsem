"""Verify expected guest services are running after boot."""

import pytest

pytestmark = pytest.mark.guest


class TestGuestServices:

    def test_pty_agent_running(self, guest_env):
        """capsem-pty-agent process is running in guest."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "pgrep -f capsem-pty-agent || pgrep -f pty.agent"})
        assert resp is not None
        stdout = resp.get("stdout", "").strip()
        assert len(stdout) > 0, "capsem-pty-agent not found running"

    def test_net_proxy_running(self, guest_env):
        """capsem-net-proxy process is running in guest."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "pgrep -f capsem-net-proxy || pgrep -f net.proxy"})
        assert resp is not None
        stdout = resp.get("stdout", "").strip()
        assert len(stdout) > 0, "capsem-net-proxy not found running"

    def test_dnsmasq_running(self, guest_env):
        """dnsmasq DNS resolver is running in guest."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "pgrep dnsmasq"})
        assert resp is not None
        stdout = resp.get("stdout", "").strip()
        assert len(stdout) > 0, "dnsmasq not found running"
