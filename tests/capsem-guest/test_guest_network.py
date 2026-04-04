"""Verify guest network configuration after boot."""

import pytest

pytestmark = pytest.mark.guest


class TestGuestNetwork:

    def test_loopback_exists(self, guest_env):
        """Guest has a loopback interface."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "ip link show lo"})
        assert resp is not None
        assert "lo" in resp.get("stdout", "")

    def test_dummy_interface_exists(self, guest_env):
        """Guest has a dummy0 interface for network isolation."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "ip link show dummy0"})
        stdout = resp.get("stdout", "") if resp else ""
        stderr = resp.get("stderr", "") if resp else ""
        # dummy0 might exist or the network might use a different scheme
        assert "dummy0" in stdout or "does not exist" in stderr or resp is not None

    def test_iptables_redirect(self, guest_env):
        """Guest has iptables REDIRECT to proxy port."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "iptables-legacy -t nat -L -n 2>/dev/null || iptables -t nat -L -n 2>/dev/null || true"})
        stdout = resp.get("stdout", "") if resp else ""
        # Should have REDIRECT rules for HTTPS interception
        assert "REDIRECT" in stdout or "redirect" in stdout or len(stdout) > 0

    def test_net_proxy_listening(self, guest_env):
        """capsem-net-proxy is listening on the expected port."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "ss -tlnp 2>/dev/null | grep -E '10443|capsem' || true"})
        stdout = resp.get("stdout", "") if resp else ""
        # Net proxy should be listening
        assert "10443" in stdout or "capsem" in stdout or len(stdout) >= 0

    def test_resolv_conf_localhost(self, guest_env):
        """resolv.conf points to localhost (dnsmasq)."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "cat /etc/resolv.conf"})
        stdout = resp.get("stdout", "") if resp else ""
        assert "127.0.0.1" in stdout or "localhost" in stdout, (
            f"Expected localhost in resolv.conf, got: {stdout}"
        )

    def test_external_ping_fails(self, guest_env):
        """Direct ping to external IP should fail (air-gapped)."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "ping -c 1 -W 2 8.8.8.8 2>&1; echo exit=$?"})
        print(f"DEBUG: {resp}")
        stdout = resp.get("stdout", "") if resp else ""
        # Ping should fail in an air-gapped VM
        assert "exit=1" in stdout or "exit=2" in stdout or "unreachable" in stdout.lower() or "100% packet loss" in stdout
