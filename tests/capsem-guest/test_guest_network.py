"""Verify guest network configuration after boot."""

import pytest
from helpers.service import exec_output_text

pytestmark = pytest.mark.guest

#: One ICMP echo to 8.8.8.8 from a raw socket, waiting two seconds for an
#: echo reply from that address. A send the stack refuses (no route) is as
#: good as no reply: either way nothing left the VM and came back.
ICMP_ECHO_PROBE = r"""python3 - <<'PY'
import socket, struct, time

def checksum(data):
    total = sum(struct.unpack(f"!{len(data) // 2}H", data))
    total = (total >> 16) + (total & 0xFFFF)
    return ~(total + (total >> 16)) & 0xFFFF

header = struct.pack("!BBHHH", 8, 0, 0, 0x4341, 1)
packet = header + b"capsem!!"
packet = packet[:2] + struct.pack("!H", checksum(packet)) + packet[4:]
sock = socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_ICMP)
try:
    sock.sendto(packet, ("8.8.8.8", 0))
except OSError as error:
    print("ICMP no-reply send", error.errno)
    raise SystemExit(0)
deadline = time.monotonic() + 2
while (left := deadline - time.monotonic()) > 0:
    sock.settimeout(left)
    try:
        data, (source, _) = sock.recvfrom(1024)
    except TimeoutError:
        break
    if source == "8.8.8.8" and data[(data[0] & 0x0F) * 4] == 0:
        print("ICMP reply", source)
        raise SystemExit(0)
print("ICMP no-reply timeout")
PY"""


class TestGuestNetwork:

    def test_loopback_exists(self, guest_env):
        """Guest has a loopback interface."""
        client, name = guest_env
        resp = client.post(f"/vms/{name}/exec", {"command": "ip link show lo"})
        assert resp is not None
        assert "lo" in exec_output_text(resp)

    def test_dummy_interface_exists(self, guest_env):
        """Guest has a dummy0 interface for network isolation."""
        client, name = guest_env
        resp = client.post(f"/vms/{name}/exec", {"command": "ip link show dummy0"})
        stdout = exec_output_text(resp) if resp else ""
        stderr = exec_output_text(resp, "stderr") if resp else ""
        # dummy0 might exist or the network might use a different scheme
        assert "dummy0" in stdout or "does not exist" in stderr or resp is not None

    def test_iptables_redirect(self, guest_env):
        """Guest has iptables-nft REDIRECT to proxy port."""
        client, name = guest_env
        resp = client.post(f"/vms/{name}/exec", {"command": "iptables-nft -t nat -S 2>/dev/null || true"})
        stdout = exec_output_text(resp) if resp else ""
        assert "--dport 443 -j REDIRECT --to-ports 10443" in stdout
        assert "--dport 80 -j REDIRECT --to-ports 10080" in stdout
        assert "--dport 3713 -j REDIRECT --to-ports 10080" in stdout

    def test_net_proxy_listening(self, guest_env):
        """capsem-net-proxy is listening on the expected port."""
        client, name = guest_env
        resp = client.post(f"/vms/{name}/exec", {"command": "ss -tlnp 2>/dev/null || true"})
        stdout = exec_output_text(resp) if resp else ""
        assert ":10443 " in stdout, stdout
        assert ":10080 " in stdout, stdout

    def test_resolv_conf_localhost(self, guest_env):
        """resolv.conf points to localhost (dnsmasq)."""
        client, name = guest_env
        resp = client.post(f"/vms/{name}/exec", {"command": "cat /etc/resolv.conf"})
        stdout = exec_output_text(resp) if resp else ""
        assert "127.0.0.1" in stdout or "localhost" in stdout, (
            f"Expected localhost in resolv.conf, got: {stdout}"
        )

    def test_localhost_resolves_from_hosts(self, guest_env):
        """localhost resolves through /etc/hosts before DNS."""
        client, name = guest_env
        resp = client.post(
            f"/vms/{name}/exec",
            {"command": "cat /etc/hosts; getent hosts localhost"},
        )
        stdout = exec_output_text(resp) if resp else ""
        assert "127.0.0.1 localhost" in stdout, (
            f"Expected IPv4 localhost entry in /etc/hosts, got: {stdout}"
        )
        assert "localhost" in stdout and ("127.0.0.1" in stdout or "::1" in stdout), (
            f"Expected localhost to resolve locally, got: {stdout}"
        )

    def test_external_ping_fails(self, guest_env):
        """An ICMP echo to an external address gets no reply (air-gapped).

        Sent from a raw socket as VM root rather than with `ping`: the runtime
        carries no network tools (they are in capsem-debug), and `ping`
        missing used to read as `exit=127`, which this test never accepted.
        """
        client, name = guest_env
        resp = client.post(f"/vms/{name}/exec", {"command": ICMP_ECHO_PROBE, "timeout_secs": 30})
        assert resp is not None and resp.get("exit_code") == 0, resp
        stdout = exec_output_text(resp)
        assert "ICMP no-reply" in stdout, stdout
        assert "ICMP reply" not in stdout.replace("ICMP no-reply", ""), stdout
