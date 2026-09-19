"""Authenticated browser previews through the real VM owner and confined router."""

import http.client
import json
import shlex
import socket
from pathlib import Path
from urllib.parse import urlencode, urlsplit

import pytest
from helpers.constants import CODE_PROFILE_ID

from tests.ironbank.kingslanding.test_publish import redis
from tests.ironbank.kingslanding.test_run import exec_output_text, service, wait_for

__all__ = ["redis", "service"]
pytestmark = pytest.mark.integration
SERVER = Path(__file__).resolve().parents[2] / "fixtures" / "preview_server.py"


def _gateway(service, method, path):
    port = int((service.tmp_dir / "gateway.port").read_text())
    token = (service.tmp_dir / "gateway.token").read_text().strip()
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    connection.request(method, path, headers={"Authorization": f"Bearer {token}"})
    response = connection.getresponse()
    body = response.read()
    connection.close()
    assert response.status < 300, (response.status, body)
    return json.loads(body) if body else None


def _preview(port, label, method, path, cookie=None, body=b"", extra=None):
    headers = {"Host": f"{label}.localhost:{port}", **(extra or {})}
    if cookie:
        headers["Cookie"] = cookie
    if body:
        headers["Content-Length"] = str(len(body))
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    connection.request(method, path, body=body, headers=headers)
    response = connection.getresponse()
    payload = response.read()
    result = response.status, response.getheaders(), payload
    connection.close()
    return result


def _refused(port, label, cookie):
    try:
        status, _, body = _preview(port, label, "GET", "/final", cookie)
        return status != 200 or body != b"redirected"
    except (ConnectionError, ConnectionResetError, http.client.HTTPException, OSError):
        return True


def _bootstrap_replay_refused(port, label, path, body):
    try:
        status, _, _ = _preview(
            port,
            label,
            "POST",
            path,
            body=body,
            extra={"Content-Type": "application/x-www-form-urlencoded"},
        )
        return status >= 400
    except (ConnectionError, ConnectionResetError, http.client.HTTPException, OSError):
        return True


def _hits(client, vm_id):
    result = client.post(
        f"/vms/{vm_id}/exec",
        {
            "command": "test ! -f /root/preview-hits && printf 0 || wc -l </root/preview-hits",
            "timeout_secs": 5,
        },
    )
    assert result["exit_code"] == 0, result
    return int(exec_output_text(result).strip())


def _start_server(service, redis):
    client = service.client()
    vm_id = redis["vm"]["id"]
    assert client.upload_file(vm_id, "preview_server.py", SERVER.read_bytes())[
        "success"
    ]
    launcher = (
        "set -eu; rm -f /root/preview-hits; "
        "pid=$(cat /var/tmp/capsem-container/workload.pid); "
        "nohup nsenter --net=/proc/$pid/ns/net -- python3 /root/preview_server.py "
        ">/root/preview-server.log 2>&1 </dev/null &"
    )
    result = client.post(
        f"/vms/{vm_id}/exec",
        {"command": launcher, "timeout_secs": 5},
    )
    assert result["exit_code"] == 0, result

    probe = "import urllib.request; assert urllib.request.urlopen('http://127.0.0.1:8080/final').read() == b'redirected'"

    def ready():
        result = client.post(
            f"/vms/{vm_id}/exec",
            {
                "command": (
                    "pid=$(cat /var/tmp/capsem-container/workload.pid); "
                    f"nsenter --net=/proc/$pid/ns/net -- python3 -c {shlex.quote(probe)}"
                ),
                "timeout_secs": 5,
            },
        )
        return result.get("exit_code") == 0

    wait_for(ready, "container preview HTTP workload", timeout=10)


def _websocket(port, label, cookie):
    connection = socket.create_connection(("127.0.0.1", port), timeout=5)
    request = (
        f"GET /live HTTP/1.1\r\nHost: {label}.localhost:{port}\r\n"
        "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n"
        "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
        f"Authorization: Bearer must-not-reach-workload\r\nCookie: {cookie}; app=client\r\n\r\n"
    )
    connection.sendall(request.encode())
    head = b""
    while b"\r\n\r\n" not in head:
        head += connection.recv(4096)
    assert head.startswith(b"HTTP/1.1 101 "), head
    payload = b"hello"
    mask = b"\x01\x02\x03\x04"
    masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
    connection.sendall(bytes([0x81, 0x80 | len(payload)]) + mask + masked)
    first, length = _recv_exact(connection, 2)
    assert first == 0x81 and length < 126
    reply = b""
    while len(reply) < length:
        reply += connection.recv(length - len(reply))
    connection.close()
    assert reply == b"echo:hello"


def _recv_exact(connection, length):
    result = b""
    while len(result) < length:
        part = connection.recv(length - len(result))
        assert part, "WebSocket closed before the frame was complete"
        result += part
    return result


def test_authenticated_container_preview_streams_and_revokes(redis, service):
    client = service.client()
    vm_id = redis["vm"]["id"]
    wait_for(
        lambda: all(
            (service.tmp_dir / name).exists()
            for name in ("gateway.port", "gateway.token", "preview.port")
        ),
        "gateway preview readiness",
        timeout=10,
    )
    _start_server(service, redis)
    exposure = client.post(
        f"/vms/{vm_id}/exposures",
        {
            "target": "container",
            "guest_port": 8080,
            "host_port": 0,
            "access": "http_preview",
        },
    )
    assert exposure["host_port"] is None and exposure["access"] == "http_preview"
    session = _gateway(
        service,
        "POST",
        f"/vms/{vm_id}/exposures/{exposure['id']}/preview-session",
    )
    url = urlsplit(session["url"])
    assert url.query == "" and url.hostname == f"{exposure['id']}.localhost"
    body = urlencode({"bootstrap_token": session["bootstrap_token"]}).encode()
    status, headers, _ = _preview(
        url.port,
        exposure["id"],
        "POST",
        url.path,
        body=body,
        extra={"Content-Type": "application/x-www-form-urlencoded"},
    )
    assert status == 303
    control_cookie = next(
        value.split(";", 1)[0]
        for name, value in headers
        if name.lower() == "set-cookie"
    )
    assert control_cookie.startswith("capsem_preview=")
    assert _bootstrap_replay_refused(url.port, exposure["id"], url.path, body)
    assert _refused(url.port, exposure["id"], "")
    assert _refused(url.port, exposure["id"], control_cookie) is False

    status, headers, body = _preview(
        url.port, exposure["id"], "GET", "/", control_cookie
    )
    assert status == 200 and b"/asset.js" in body and b"/form" in body
    cookies = [value for name, value in headers if name.lower() == "set-cookie"]
    assert any(value.startswith("app_session=guest") for value in cookies)
    assert not any(value.startswith("capsem_preview=") for value in cookies)
    assert _preview(url.port, exposure["id"], "GET", "/asset.js", control_cookie)[
        2
    ].startswith(b"window.")
    assert (
        _preview(
            url.port, exposure["id"], "POST", "/form", control_cookie, b"name=capsem"
        )[2]
        == b"form:name=capsem"
    )
    redirect = _preview(url.port, exposure["id"], "GET", "/redirect", control_cookie)
    assert redirect[0] == 302
    assert any(name.lower() == "location" and value == "/final" for name, value in redirect[1])
    assert (
        _preview(url.port, exposure["id"], "GET", "/stream", control_cookie)[2]
        == b"stream-complete"
    )
    headers_result = _preview(
        url.port,
        exposure["id"],
        "GET",
        "/headers",
        control_cookie + "; app=client",
        extra={
            "Authorization": "Bearer administrator",
            "Proxy-Authorization": "Basic secret",
        },
    )
    observed = json.loads(headers_result[2])
    assert observed == {
        "authorization": False,
        "cookie": "app=client",
        "proxy_authorization": False,
    }
    _websocket(url.port, exposure["id"], control_cookie)

    second = client.post(
        f"/vms/{vm_id}/exposures",
        {
            "target": "container",
            "guest_port": 8080,
            "host_port": 0,
            "access": "http_preview",
        },
    )
    _gateway(service, "POST", f"/vms/{vm_id}/exposures/{second['id']}/preview-session")
    before = _hits(client, vm_id)
    assert _refused(url.port, second["id"], control_cookie)
    assert _hits(client, vm_id) == before

    rule = client.put(
        f"/profiles/{CODE_PROFILE_ID}/enforcement/rules/preview_test/edit",
        {
            "name": "preview_test",
            "action": "block",
            "match": 'network.mode == "http_preview" && network.action == "preview_request"',
            "reason": "Kingslanding preview boundary proof.",
        },
    )
    assert rule["rule"]["action"] == "block"
    assert _refused(url.port, exposure["id"], control_cookie)
    assert _hits(client, vm_id) == before

    rows = []

    def denied_audit():
        rows[:] = client.get(f"/vms/{vm_id}/security/latest?limit=2000")
        return any(
            row["event_type"] == "network.connect"
            and json.loads(row["event_json"])
            .get("network", {})
            .get("route", {})
            .get("mode")
            == "preview"
            and json.loads(row["event_json"]).get("decision", {}).get("effective")
            == "block"
            for row in rows
        )

    wait_for(denied_audit, "denied preview audit", timeout=15)
    assert client.delete(f"/vms/{vm_id}/exposures/{exposure['id']}")["success"]
    assert _refused(url.port, exposure["id"], control_cookie)
    assert _hits(client, vm_id) == before
    assert client.delete(f"/vms/{vm_id}/exposures/{second['id']}")["success"]
