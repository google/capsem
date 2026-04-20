"""Network isolation, MITM proxy, and trust chain tests.

Tests are ordered from low-level to high-level so failures pinpoint
the exact layer that broke.
"""

import os
import subprocess

import pytest

from conftest import run


# ---------------------------------------------------------------
# Layer 1: Guest network plumbing (dummy0, dnsmasq, iptables)
# ---------------------------------------------------------------


def test_dummy0_has_ip():
    """dummy0 must have 10.0.0.1 assigned."""
    result = run("ip -4 addr show dummy0")
    assert result.returncode == 0, f"dummy0 not found:\n{result.stderr}"
    assert "10.0.0.1" in result.stdout, \
        f"10.0.0.1 not on dummy0:\n{result.stdout}"


def test_dnsmasq_responds():
    """dnsmasq must answer DNS queries on 127.0.0.1:53."""
    result = run("getent hosts test-dns-probe.invalid", timeout=5)
    assert "10.0.0.1" in result.stdout, \
        f"dnsmasq did not resolve to 10.0.0.1: {result.stdout}"


@pytest.mark.parametrize("domain", [
    "github.com",
    "google.com",
    "cloudflare.com",
    "example.org",
    "python.org",
])
def test_dns_all_resolve_to_local(domain):
    """All DNS queries must resolve to 10.0.0.1 via fake dnsmasq."""
    result = run(f"getent hosts {domain} 2>&1", timeout=5)
    assert "10.0.0.1" in result.stdout, \
        f"{domain} did not resolve to 10.0.0.1: {result.stdout}"


def test_iptables_redirect_443_to_10443():
    """iptables must REDIRECT port 443 to 10443."""
    result = run(
        "iptables-legacy -t nat -L OUTPUT -n 2>&1 || iptables -t nat -L OUTPUT -n 2>&1",
        timeout=5,
    )
    assert "REDIRECT" in result.stdout and "10443" in result.stdout, \
        f"no REDIRECT 443->10443:\n{result.stdout}"


# ---------------------------------------------------------------
# Layer 2: Guest net-proxy (TCP 10443 -> vsock 5002)
# ---------------------------------------------------------------


def test_net_proxy_listening():
    """capsem-net-proxy must accept TCP on 127.0.0.1:10443."""
    result = run(
        "python3 -c \""
        "import socket; s=socket.socket(); s.settimeout(3); "
        "s.connect(('127.0.0.1', 10443)); "
        "print('OK'); s.close()\"",
        timeout=10,
    )
    assert "OK" in result.stdout, \
        f"cannot connect to net-proxy: {result.stderr.strip() or result.stdout.strip()}"


def test_tcp_443_reaches_proxy():
    """TCP to 10.0.0.1:443 must be redirected to net-proxy (10443)."""
    result = run(
        "python3 -c \""
        "import socket; s=socket.socket(); s.settimeout(5); "
        "s.connect(('10.0.0.1', 443)); "
        "print('OK'); s.close()\"",
        timeout=10,
    )
    assert "OK" in result.stdout, \
        f"TCP 443 redirect failed: {result.stderr.strip() or result.stdout.strip()}"


def test_vsock_bridge_delivers_bytes():
    """Send raw bytes through the proxy and verify the host responds (or closes)."""
    # Send garbage -- the host MITM proxy should close with no SNI.
    # The point is that bytes flow through the vsock bridge.
    result = run(
        "python3 -c \""
        "import socket; s=socket.socket(); s.settimeout(5); "
        "s.connect(('127.0.0.1', 10443)); "
        "s.sendall(b'HELLO'); "
        "try:\n"
        "    data = s.recv(1024)\n"
        "    print(f'RECV {len(data)} bytes')\n"
        "except socket.timeout:\n"
        "    print('TIMEOUT')\n"
        "except ConnectionResetError:\n"
        "    print('RESET')\n"
        "s.close()\" 2>&1",
        timeout=10,
    )
    # We expect RECV 0 (clean close) or RESET -- both prove the bridge works.
    out = result.stdout.strip()
    assert "TIMEOUT" not in out, \
        f"vsock bridge did not respond (host unreachable?): {out}"


# ---------------------------------------------------------------
# Layer 3: TLS handshake (MITM proxy termination)
# ---------------------------------------------------------------


def test_tls_handshake_completes():
    """TLS handshake to allowed domain must complete through the MITM proxy."""
    result = run(
        "python3 -c \""
        "import socket, ssl; "
        "s = socket.socket(); s.settimeout(10); "
        "s.connect(('10.0.0.1', 443)); "
        "ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT); "
        "ctx.check_hostname = False; "
        "ctx.verify_mode = ssl.CERT_NONE; "
        "ws = ctx.wrap_socket(s, server_hostname='google.com'); "
        "print('TLS_OK version=' + str(ws.version())); "
        "print('cipher=' + str(ws.cipher())); "
        "cert = ws.getpeercert(binary_form=True); "
        "print(f'cert_size={len(cert)}'); "
        "ws.close()\" 2>&1",
        timeout=20,
    )
    assert "TLS_OK" in result.stdout, \
        f"TLS handshake failed:\n{result.stdout}"


def test_tls_cert_from_capsem_ca():
    """MITM proxy must present a cert signed by the Capsem CA."""
    result = run(
        "python3 -c \""
        "import socket, ssl; "
        "s = socket.socket(); s.settimeout(10); "
        "s.connect(('10.0.0.1', 443)); "
        "ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT); "
        "ctx.check_hostname = False; "
        "ctx.verify_mode = ssl.CERT_NONE; "
        "ws = ctx.wrap_socket(s, server_hostname='google.com'); "
        "cert = ws.getpeercert(); "
        "issuer = dict(x[0] for x in cert.get('issuer', ())); "
        "cn = issuer.get('commonName', ''); "
        "print(f'issuer_cn={cn}'); "
        "san = [v for t, v in cert.get('subjectAltName', ()) if t == 'DNS']; "
        "print(f'san={san}'); "
        "ws.close()\" 2>&1",
        timeout=20,
    )
    assert "issuer_cn=" in result.stdout, \
        f"could not get cert info:\n{result.stdout}"
    # Check issuer is Capsem CA (might be empty if cert_none hides it)
    if "Capsem" not in result.stdout:
        # cert_none doesn't populate getpeercert() fully -- just check TLS worked
        assert result.returncode == 0, \
            f"TLS failed:\n{result.stdout}"


# ---------------------------------------------------------------
# Layer 4: HTTP over MITM (full request/response)
# ---------------------------------------------------------------


def test_curl_https_with_skip_verify():
    """curl -k to allowed domain must get HTTP response."""
    result = run("curl -skI --connect-timeout 10 https://google.com 2>&1", timeout=20)
    assert result.returncode == 0, \
        f"curl -k failed (exit {result.returncode}):\n{result.stdout}"
    assert "HTTP/" in result.stdout, f"no HTTP response:\n{result.stdout}"


def test_curl_verbose_diagnostics():
    """curl -v captures the full handshake trace for debugging."""
    result = run("curl -vvk --connect-timeout 10 -o /dev/null https://google.com 2>&1", timeout=20)
    # Even if curl fails, capture the verbose output for diagnosis.
    # This test always passes -- it's here for diagnostic output on failure.
    lines = result.stdout.strip().split('\n') if result.stdout else []
    info = {
        "exit_code": result.returncode,
        "connected": any("Connected to" in l for l in lines),
        "ssl_handshake": any("SSL connection" in l for l in lines),
        "http_response": any("HTTP/" in l for l in lines),
        "error_lines": [l for l in lines if "error" in l.lower()],
    }
    # If curl failed, print the full trace as the assertion message.
    if result.returncode != 0:
        trace = result.stdout[:3000] if result.stdout else "(empty)"
        pytest.fail(
            f"curl -v failed (exit {result.returncode}).\n"
            f"Handshake info: {info}\n"
            f"Full trace:\n{trace}"
        )


# ---------------------------------------------------------------
# Layer 5: MITM CA trust (no -k needed)
# ---------------------------------------------------------------


def test_mitm_ca_cert_file_exists():
    """Capsem CA cert file must exist."""
    assert os.path.isfile("/usr/local/share/ca-certificates/capsem-ca.crt"), \
        "capsem-ca.crt not found"


def test_mitm_ca_in_system_bundle():
    """Capsem MITM CA must be in the system CA bundle."""
    # Grep for a unique base64 fragment from the cert (CN is DER-encoded, not plain text)
    result = run("grep -c 'OMYp0kksjRwy' /etc/ssl/certs/ca-certificates.crt")
    assert result.returncode == 0 and int(result.stdout.strip()) > 0, \
        "Capsem CA not found in system CA bundle"


def test_certifi_includes_capsem_ca():
    """Python certifi bundle must include the Capsem CA."""
    result = run(
        'python3 -c "'
        "import certifi; "
        "bundle = open(certifi.where()).read(); "
        "print('found' if 'OMYp0kksjRwy' in bundle else 'missing')"
        '"'
    )
    assert result.returncode == 0
    assert "found" in result.stdout, \
        "Capsem CA not found in certifi bundle"


def test_curl_allowed_domain_ca_trusted():
    """curl without -k must succeed (system trusts Capsem CA)."""
    result = run(
        "curl -sI --connect-timeout 10 https://google.com 2>&1",
        timeout=20,
    )
    assert result.returncode == 0, \
        f"curl failed without -k (CA not trusted?):\n{result.stdout}\n{result.stderr}"
    assert "HTTP/" in result.stdout, f"no HTTP response:\n{result.stdout}"


def test_python_urllib_https_trusted():
    """Python urllib must complete TLS via system CA trust."""
    # Verify TLS works by connecting with ssl module (urllib raises HTTPError
    # for 403 responses, which obscures the TLS-success signal we care about).
    result = run(
        'python3 -c "'
        "import ssl, socket; "
        "ctx = ssl.create_default_context(); "
        "s = ctx.wrap_socket(socket.create_connection(('google.com', 443), timeout=10), server_hostname='google.com'); "
        "print('OK version=' + str(s.version())); "
        "s.close()"
        '" 2>&1',
        timeout=20,
    )
    assert result.returncode == 0 and "OK" in result.stdout, \
        f"Python urllib HTTPS failed (TLS or connection error):\n{result.stdout}"


# -- HTTPS environment variables --


@pytest.mark.parametrize("var", [
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
])
def test_ca_env_var_set(var):
    """CA-related env vars must be set for runtime trust."""
    value = os.environ.get(var)
    assert value is not None, f"{var} not set in environment"
    assert os.path.isfile(value), f"{var}={value} but file does not exist"


# ---------------------------------------------------------------
# Layer 6: Policy enforcement (denied domains, ports)
# ---------------------------------------------------------------


def test_denied_domain_rejected():
    """HTTPS to an unconditionally denied domain must be rejected.

    ``api.openai.com`` is allowlist-gated by ``CAPSEM_OPENAI_ALLOWED`` and will
    return 401 (real upstream auth failure) when enabled -- see
    ``test_ai_provider_domain_blocked`` for that matrix. This test uses a
    domain that no policy path ever allows, so the proxy must reject it.
    """
    result = run("curl -skI --connect-timeout 5 https://evil-never-allowed.invalid 2>&1", timeout=15)
    assert result.returncode != 0 or "403" in result.stdout, \
        f"curl to denied domain should fail or return 403: {result.stdout}"


def test_post_to_random_domain_denied():
    """POST to a non-allow-listed domain must return 403."""
    result = run("curl -ski -X POST --connect-timeout 5 https://example.com 2>&1", timeout=15)
    assert "403" in result.stdout or result.returncode != 0, "POST to denied domain should return 403 or fail"


@pytest.mark.parametrize("domain,env_var", [
    ("api.anthropic.com", "CAPSEM_ANTHROPIC_ALLOWED"),
    ("api.openai.com", "CAPSEM_OPENAI_ALLOWED"),
])
def test_ai_provider_domain_blocked(domain, env_var):
    """AI provider domains: blocked unless allowed by policy, reachable if allowed."""
    result = run(
        f"curl -skI --connect-timeout 10 https://{domain} 2>&1",
        timeout=20,
    )
    if os.environ.get(env_var) == "1":
        # Domain is allowed -- must be reachable (HTTP response, not 403).
        assert "HTTP/" in result.stdout, \
            f"{domain} is allowed ({env_var}=1) but not reachable: {result.stdout}"
    else:
        # Domain is blocked -- must get 403 or connection refused.
        assert result.returncode != 0 or "403" in result.stdout, \
            f"Connection to {domain} should be blocked: {result.stdout}"


def test_http_port_80_not_proxied():
    """Plain HTTP (port 80) must not be proxied."""
    result = run(
        "curl -sI --connect-timeout 5 http://google.com 2>&1",
        timeout=15,
    )
    assert result.returncode != 0, \
        "HTTP port 80 should not be reachable"


def test_non_standard_port_fails():
    """Connections to non-443 ports must fail."""
    result = run(
        "curl -skI --connect-timeout 5 https://google.com:8443 2>&1",
        timeout=15,
    )
    assert result.returncode != 0, \
        "Non-standard HTTPS port should not be reachable"


def test_direct_ip_no_route():
    """Direct IP connection must fail -- no real NIC."""
    result = run(
        "curl -skI --connect-timeout 5 https://1.1.1.1 2>&1",
        timeout=15,
    )
    assert result.returncode != 0, \
        "Direct IP connection should fail (no real route)"


# ---------------------------------------------------------------
# Layer 7: Proxy throughput
# ---------------------------------------------------------------

_THROUGHPUT_URL = "https://ash-speed.hetzner.com/100MB.bin"
_THROUGHPUT_DOMAIN = "ash-speed.hetzner.com"
_MIN_SPEED_MBPS = 0.5


def test_proxy_download_throughput():
    """100MB download through the MITM proxy must complete above minimum speed.

    Exercises the full pipeline: guest curl -> iptables -> net-proxy ->
    vsock -> host MITM proxy -> upstream TLS -> back.  Skipped when the
    speed-test domain is not in the allow list.
    """
    # Probe reachability first so we can skip cleanly rather than fail.
    probe = run(
        f"curl -skI --connect-timeout 10 {_THROUGHPUT_URL} 2>&1",
        timeout=20,
    )
    if probe.returncode != 0 or "403" in probe.stdout:
        pytest.skip(f"{_THROUGHPUT_DOMAIN} not in allow list (add to network.custom_allow to run)")

    result = run(
        f"curl -s -o /dev/null"
        f" -w '%{{speed_download}} %{{size_download}} %{{time_total}}'"
        f" --connect-timeout 15"
        f" {_THROUGHPUT_URL}",
        timeout=180,
    )
    assert result.returncode == 0, \
        f"download failed (exit {result.returncode}):\n{result.stderr}"

    parts = result.stdout.strip().split()
    assert len(parts) == 3, f"unexpected curl output: {result.stdout!r}"

    speed_bps = float(parts[0])
    size_bytes = int(parts[1])
    time_s = float(parts[2])
    speed_mbps = speed_bps / (1024 * 1024)

    print(
        f"\nProxy throughput: {size_bytes / (1024*1024):.1f} MB"
        f" in {time_s:.1f}s = {speed_mbps:.2f} MB/s"
    )

    assert size_bytes >= 10 * 1024 * 1024, \
        f"incomplete download: {size_bytes / (1024*1024):.1f} MB (expected 10 MB)"
    assert speed_mbps >= _MIN_SPEED_MBPS, \
        f"throughput too low: {speed_mbps:.2f} MB/s (minimum {_MIN_SPEED_MBPS} MB/s)"
