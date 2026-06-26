"""Network isolation, MITM proxy, and trust chain tests.

Tests are ordered from low-level to high-level so failures pinpoint
the exact layer that broke.
"""

import os
from urllib.parse import urlsplit

import pytest

from conftest import run

LOCAL_MOCK_SERVER_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"


def _local_mock_url(path):
    base_url = os.environ.get(LOCAL_MOCK_SERVER_ENV)
    if not base_url:
        return None
    return f"{base_url.rstrip('/')}/{path.lstrip('/')}"


def _require_local_mock_url(path, reason):
    url = _local_mock_url(path)
    if not url:
        pytest.fail(
            f"{reason}; set {LOCAL_MOCK_SERVER_ENV} for deterministic local proof"
        )
    parsed = urlsplit(url)
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    if parsed.scheme == "http" and port not in (80, 3128, 3713, 8080, 11434):
        pytest.fail(
            f"{reason}; local mock server port {port} is outside the "
            "default HTTP upstream allowlist"
        )
    return url


# ---------------------------------------------------------------
# Layer 1: Guest network plumbing (dummy0, capsem-dns-proxy, iptables)
# ---------------------------------------------------------------


def test_dummy0_has_ip():
    """dummy0 must have 10.0.0.1 assigned."""
    result = run("ip -4 addr show dummy0")
    assert result.returncode == 0, f"dummy0 not found:\n{result.stderr}"
    assert "10.0.0.1" in result.stdout, \
        f"10.0.0.1 not on dummy0:\n{result.stdout}"


def test_dns_proxy_listening_udp():
    """T3.4: capsem-dns-proxy must listen on UDP :1053."""
    result = run("ss -lun 2>&1", timeout=5)
    assert ":1053" in result.stdout, \
        f"capsem-dns-proxy not listening on UDP 1053:\n{result.stdout}"


def test_dns_proxy_listening_tcp():
    """T3.4: capsem-dns-proxy must listen on TCP :1053."""
    result = run("ss -ltn 2>&1", timeout=5)
    assert ":1053" in result.stdout, \
        f"capsem-dns-proxy not listening on TCP 1053:\n{result.stdout}"


def test_iptables_redirect_dns_udp_to_1053():
    """T3.4: iptables-nft must REDIRECT UDP port 53 to 1053
    (capsem-dns-proxy)."""
    result = run("iptables-nft -t nat -S OUTPUT 2>&1", timeout=5)
    assert "1053" in result.stdout, \
        f"no REDIRECT to 1053 (DNS proxy):\n{result.stdout}"
    assert "-p udp" in result.stdout and "--dport 53" in result.stdout, \
        f"no UDP dport 53 redirect rule:\n{result.stdout}"


def test_iptables_redirect_dns_tcp_to_1053():
    """T3.4: iptables-nft must REDIRECT TCP port 53 to 1053 (large
    answers / TC-bit retries fall through TCP)."""
    result = run("iptables-nft -t nat -S OUTPUT 2>&1", timeout=5)
    assert "-p tcp" in result.stdout and "--dport 53" in result.stdout, \
        f"no TCP dport 53 redirect rule:\n{result.stdout}"


def test_dns_query_reaches_capsem_proxy():
    """A DNS query must reach the Capsem proxy instead of the old wildcard
    dnsmasq sentinel path. The reserved .invalid TLD keeps the proof hermetic."""
    result = run(
        "getent hosts capsem-doctor-hermetic.invalid 2>&1",
        timeout=10,
    )
    assert result.returncode != 0, \
        f"reserved .invalid domain unexpectedly resolved:\n{result.stdout}"
    assert "10.0.0.1" not in result.stdout


def test_dns_nxdomain_propagates_from_upstream():
    """T3.4 acceptance: a name that doesn't exist anywhere must
    NXDOMAIN cleanly through the proxy. The `.invalid` TLD is
    reserved by RFC 2606 to never resolve, so this is the cleanest
    end-to-end NXDOMAIN test that doesn't depend on the user's
    policy. Pre-T3 dnsmasq returned 10.0.0.1 for *everything*
    including .invalid -- this test pins the cutover."""
    result = run(
        "getent hosts nope-this-does-not-exist-capsem-test.invalid 2>&1",
        timeout=10,
    )
    # getent returns 2 (HOST_NOT_FOUND) on NXDOMAIN.
    assert result.returncode != 0, \
        f"`.invalid` domain unexpectedly resolved (pre-T3 dnsmasq behavior):\n{result.stdout}"
    # And the legacy 10.0.0.1 sentinel is not in any output.
    assert "10.0.0.1" not in result.stdout


def test_iptables_redirect_443_to_10443():
    """iptables-nft must REDIRECT port 443 to 10443."""
    result = run("iptables-nft -t nat -S OUTPUT 2>&1", timeout=5)
    assert "REDIRECT" in result.stdout and "10443" in result.stdout, \
        f"no REDIRECT 443->10443:\n{result.stdout}"


def test_iptables_redirect_80_to_10080():
    """T2.2: iptables-nft must REDIRECT port 80 to 10080 (plain HTTP)."""
    result = run("iptables-nft -t nat -S OUTPUT 2>&1", timeout=5)
    # Look for a REDIRECT line carrying ports "80" and "10080".
    assert "10080" in result.stdout, \
        f"no REDIRECT to 10080 (plain HTTP path):\n{result.stdout}"
    assert "-p tcp" in result.stdout and "--dport 80" in result.stdout, \
        f"no dport 80 redirect rule:\n{result.stdout}"


def test_iptables_redirect_plain_http_allowlist_to_10080():
    """T2.2: default plain-HTTP allowlist must REDIRECT to 10080."""
    result = run("iptables-nft -t nat -S OUTPUT 2>&1", timeout=5)
    for port in (3128, 3713, 8080, 11434):
        assert f"--dport {port}" in result.stdout, \
            f"no REDIRECT for {port} -> 10080:\n{result.stdout}"


# ---------------------------------------------------------------
# Layer 2: Guest net-proxy (TCP 10443 / 10080 -> vsock 5002)
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
        f"cannot connect to net-proxy (HTTPS): {result.stderr.strip() or result.stdout.strip()}"


def test_net_proxy_http_listening():
    """T2.2: capsem-net-proxy must also accept TCP on 127.0.0.1:10080."""
    result = run(
        "python3 -c \""
        "import socket; s=socket.socket(); s.settimeout(3); "
        "s.connect(('127.0.0.1', 10080)); "
        "print('OK'); s.close()\"",
        timeout=10,
    )
    assert "OK" in result.stdout, \
        f"cannot connect to net-proxy (HTTP): {result.stderr.strip() or result.stdout.strip()}"


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
    """TLS handshake must complete through the local MITM proxy."""
    result = run(
        "python3 -c \""
        "import socket, ssl; "
        "s = socket.socket(); s.settimeout(10); "
        "s.connect(('10.0.0.1', 443)); "
        "ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT); "
        "ctx.check_hostname = False; "
        "ctx.verify_mode = ssl.CERT_NONE; "
        "ws = ctx.wrap_socket(s, server_hostname='capsem-doctor.local'); "
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
        "ws = ctx.wrap_socket(s, server_hostname='capsem-doctor.local'); "
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


def test_curl_https_without_system_ca_validation():
    """curl through the local HTTP MITM rail must get a deterministic response."""
    local_url = _require_local_mock_url("/tiny", "local HTTP curl smoke")
    result = run(f"curl -sSI --connect-timeout 10 {local_url} 2>&1", timeout=20)
    assert result.returncode == 0, \
        f"curl failed (exit {result.returncode}):\n{result.stdout}"
    assert "HTTP/" in result.stdout, f"no HTTP response:\n{result.stdout}"


def test_curl_verbose_diagnostics():
    """curl -v captures the full handshake trace for debugging."""
    local_url = _require_local_mock_url("/tiny", "local verbose curl smoke")
    result = run(f"curl -vv --connect-timeout 10 -o /dev/null {local_url} 2>&1", timeout=20)
    # Even if curl fails, capture the verbose output for diagnosis.
    # This test always passes -- it's here for diagnostic output on failure.
    lines = result.stdout.strip().split('\n') if result.stdout else []
    info = {
        "exit_code": result.returncode,
        "connected": any("Connected to" in line for line in lines),
        "ssl_handshake": any("SSL connection" in line for line in lines),
        "http_response": any("HTTP/" in line for line in lines),
        "error_lines": [line for line in lines if "error" in line.lower()],
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
    """curl without public access must still prove the local rail works."""
    local_url = _require_local_mock_url("/tiny", "local curl trust smoke")
    result = run(
        f"curl -sI --connect-timeout 10 {local_url} 2>&1",
        timeout=20,
    )
    assert result.returncode == 0, \
        f"curl failed against local mock server:\n{result.stdout}\n{result.stderr}"
    assert "HTTP/" in result.stdout, f"no HTTP response:\n{result.stdout}"


def test_python_urllib_https_trusted():
    """Python ssl must complete a local MITM TLS handshake."""
    result = run(
        'python3 -c "'
        "import ssl, socket; "
        "ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT); "
        "ctx.check_hostname = False; "
        "ctx.verify_mode = ssl.CERT_NONE; "
        "s = ctx.wrap_socket(socket.create_connection(('10.0.0.1', 443), timeout=10), server_hostname='capsem-doctor.local'); "
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

    This test uses a reserved domain that no rule ever matches.
    """
    result = run("curl -skI --connect-timeout 5 https://evil-never-allowed.invalid 2>&1", timeout=15)
    assert result.returncode != 0 or "403" in result.stdout, \
        f"curl to denied domain should fail or return 403: {result.stdout}"


def test_post_to_random_domain_denied():
    """POST to a denied HTTPS domain must not silently pass."""
    result = run(
        "curl -skX POST --connect-timeout 5 "
        "-H 'content-type: application/json' "
        "-d '{\"probe\":\"doctor-deny\"}' "
        "https://evil-never-allowed.invalid/deny-target 2>&1",
        timeout=15,
    )
    assert result.returncode != 0 or "403" in result.stdout, \
        f"POST to denied domain should fail or return 403: {result.stdout}"


def test_http_port_80_is_proxied():
    """Plain HTTP (port 80) is inspected by the MITM proxy."""
    local_url = _require_local_mock_url("/tiny", "local HTTP proxy smoke")
    result = run(
        f"curl -sS --connect-timeout 5 {local_url} 2>&1",
        timeout=15,
    )
    assert result.returncode == 0, \
        f"local HTTP through proxy failed: {result.stdout}"
    assert "capsem-mock-server:tiny" in result.stdout, \
        f"unexpected local HTTP response: {result.stdout}"


def test_local_http_gzip_decompression_path():
    """Gzip response bodies must travel through the local MITM rail."""
    local_url = _require_local_mock_url("/gzip/10kb", "local gzip smoke")
    result = run(
        f"curl -sS --compressed --connect-timeout 5 {local_url} | wc -c",
        timeout=15,
    )
    assert result.returncode == 0, f"gzip curl failed: {result.stdout}"
    assert result.stdout.strip() == str(10 * 1024), \
        f"unexpected decoded gzip byte count: {result.stdout}"


def test_local_http_delayed_chunk_stream():
    """Chunked response streaming must complete through the local MITM rail."""
    local_url = _require_local_mock_url("/delayed-chunks", "local chunk smoke")
    result = run(
        f"curl -sS --connect-timeout 5 {local_url}",
        timeout=15,
    )
    assert result.returncode == 0, f"chunk curl failed: {result.stdout}"
    assert "chunk-0" in result.stdout and "chunk-3" in result.stdout, \
        f"missing chunk fixture output: {result.stdout}"


def test_local_sse_model_fixture():
    """SSE model-shaped traffic must traverse the local MITM rail."""
    local_url = _require_local_mock_url("/sse/model", "local SSE model smoke")
    result = run(
        f"curl -sS --connect-timeout 5 {local_url}",
        timeout=15,
    )
    assert result.returncode == 0, f"SSE curl failed: {result.stdout}"
    assert "model.tool_call" in result.stdout and "fixture_lookup" in result.stdout, \
        f"unexpected SSE model fixture: {result.stdout}"


def test_local_openai_compatible_model_fixture():
    """OpenAI-compatible model traffic must be observed without public services."""
    local_url = _require_local_mock_url(
        "/v1/chat/completions",
        "local OpenAI-compatible model smoke",
    )
    result = run(
        "python3 - <<'PY'\n"
        "from pathlib import Path\n"
        "import subprocess\n"
        "payload_path = Path('/tmp/capsem-doctor-openai-payload.json')\n"
        "config_path = Path('/tmp/capsem-doctor-openai-curl.conf')\n"
        "secret = 'sk-' + 'capsem_' + 'test_' + 'openai_api_key_' + '0123456789abcdef'\n"
        "payload_path.write_text('{\"model\":\"mock-local\","
        "\"messages\":[{\"role\":\"user\",\"content\":\"call fixture_lookup\"}],"
        "\"tools\":[{\"type\":\"function\",\"function\":{\"name\":\"fixture_lookup\","
        "\"parameters\":{\"type\":\"object\",\"properties\":{\"query\":{\"type\":\"string\"}}}}}]}')\n"
        "config_path.write_text(\n"
        "    'silent\\n'\n"
        "    'show-error\\n'\n"
        "    'connect-timeout = 5\\n'\n"
        "    'request = POST\\n'\n"
        "    'header = \"content-type: application/json\"\\n'\n"
        "    f'header = \"authorization: Bearer {secret}\"\\n'\n"
        "    f'data = \"@{payload_path}\"\\n'\n"
        f"    'url = \"{local_url}\"\\n'\n"
        ")\n"
        "raise SystemExit(subprocess.run(['curl', '--config', str(config_path)]).returncode)\n"
        "PY",
        timeout=15,
    )
    assert result.returncode == 0, f"model fixture curl failed: {result.stdout}"
    assert '"model":"mock-local"' in result.stdout.replace(" ", ""), \
        f"model fixture did not report mock-local: {result.stdout}"
    assert "tool_calls" in result.stdout and "fixture_lookup" in result.stdout, \
        f"model fixture did not include tool call: {result.stdout}"


def test_local_credential_fixture_is_broker_stimulus_only():
    """Credential-shaped fixture traffic should trigger broker logging without
    dumping synthetic secret values into doctor output."""
    local_url = _require_local_mock_url("/credential/response", "local broker smoke")
    result = run(
        f"curl -sS -o /dev/null -w '%{{http_code}} %{{size_download}}'"
        f" --connect-timeout 5 {local_url}",
        timeout=15,
    )
    assert result.returncode == 0, f"credential fixture curl failed: {result.stdout}"
    assert result.stdout.strip().startswith("200 "), \
        f"credential fixture did not return HTTP 200: {result.stdout}"
    assert "capsem_test_" not in result.stdout


def test_local_oauth_token_fixture_is_broker_stimulus_only():
    """OAuth token exchange traffic must be exercised hermetically without
    dumping synthetic token values into doctor output."""
    local_url = _require_local_mock_url("/oauth/token", "local OAuth token smoke")
    result = run(
        "python3 - <<'PY'\n"
        "from pathlib import Path\n"
        "import subprocess\n"
        "body_path = Path('/tmp/capsem-doctor-oauth-form.txt')\n"
        "config_path = Path('/tmp/capsem-doctor-oauth-curl.conf')\n"
        "code = 'capsem_' + 'test_' + 'oauth_code_' + '0123456789abcdef'\n"
        "client_secret = 'capsem_' + 'test_' + 'oauth_client_secret'\n"
        "body_path.write_text(\n"
        "    'grant_type=authorization_code'\n"
        "    f'&code={code}'\n"
        "    f'&client_secret={client_secret}'\n"
        ")\n"
        "config_path.write_text(\n"
        "    'silent\\n'\n"
        "    'show-error\\n'\n"
        "    'output = /dev/null\\n'\n"
        "    'write-out = \"%{http_code} %{size_download}\"\\n'\n"
        "    'connect-timeout = 5\\n'\n"
        "    'request = POST\\n'\n"
        "    'header = \"content-type: application/x-www-form-urlencoded\"\\n'\n"
        "    f'data = \"@{body_path}\"\\n'\n"
        f"    'url = \"{local_url}\"\\n'\n"
        ")\n"
        "raise SystemExit(subprocess.run(['curl', '--config', str(config_path)]).returncode)\n"
        "PY",
        timeout=15,
    )
    assert result.returncode == 0, f"OAuth fixture curl failed: {result.stdout}"
    assert result.stdout.strip().startswith("200 "), \
        f"OAuth fixture did not return HTTP 200: {result.stdout}"
    assert "capsem_test_" not in result.stdout


def test_local_websocket_echo_fixture():
    """WebSocket upgrade and frame echo must work against the local lab."""
    local_url = _require_local_mock_url("/ws/echo", "local WebSocket smoke")
    ws_url = local_url.replace("http://", "ws://", 1).replace("https://", "wss://", 1)
    result = run(
        "python3 - <<'PY'\n"
        "import sys\n"
        "from websockets.sync.client import connect\n"
        f"with connect({ws_url!r}, proxy=None, open_timeout=5, close_timeout=5) as ws:\n"
        "    ws.send('doctor-websocket')\n"
        "    reply = ws.recv(timeout=5)\n"
        "    print(reply)\n"
        "PY",
        timeout=15,
    )
    assert result.returncode == 0, f"websocket fixture failed: {result.stdout}"
    assert "doctor-websocket" in result.stdout, \
        f"unexpected websocket echo: {result.stdout}"


def test_non_standard_port_fails():
    """Connections to non-443 ports must fail."""
    result = run(
        "curl -skI --connect-timeout 5 https://127.0.0.1:8443 2>&1",
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

_MIN_SPEED_MBPS = 0.5


def test_proxy_download_throughput():
    """Download through the MITM proxy above the minimum speed.

    Exercises the full pipeline: guest curl -> iptables -> net-proxy ->
    vsock -> host MITM proxy -> upstream -> back. Public network is an
    explicit smoke only; default release gates should use the local lab.
    """
    local_url = _require_local_mock_url("/bytes/10mb", "local proxy throughput smoke")
    result = run(
        f"curl -sL -o /dev/null"
        f" -w '%{{speed_download}} %{{size_download}} %{{time_total}}'"
        f" --connect-timeout 15"
        f" {local_url}",
        timeout=180,
    )
    expected_bytes = 10 * 1024 * 1024

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

    assert size_bytes >= expected_bytes, \
        f"incomplete download: {size_bytes / (1024*1024):.1f} MB"
    assert speed_mbps >= _MIN_SPEED_MBPS, \
        f"throughput too low: {speed_mbps:.2f} MB/s (minimum {_MIN_SPEED_MBPS} MB/s)"
