"""Ironbank contract for the one reusable local mock server.

The release rail should not grow per-feature fake upstreams. This test starts
the shared mock server once and proves the advertised protocol surfaces are real
enough for doctor, benchmarks, and model/client ledger tests to depend on.
"""

from __future__ import annotations

import json
import socket
import struct
from pathlib import Path
from urllib.request import Request, urlopen

import pytest

from helpers.mock_server import start_mock_server, stop_process


pytestmark = pytest.mark.integration


def _post_json(url: str, value: object) -> dict:
    request = Request(
        url,
        data=json.dumps(value).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urlopen(request, timeout=5) as response:
        assert response.status == 200
        assert response.headers["content-type"] in {"application/json", "text/event-stream"}
        body = response.read().decode()
    if body.startswith("event:") or body.startswith("data:"):
        return {"_stream": body}
    parsed = json.loads(body)
    assert isinstance(parsed, dict)
    return parsed


def _dns_query(name: str, query_id: int = 0xCAFE) -> bytes:
    labels = b"".join(bytes([len(part)]) + part.encode("ascii") for part in name.split("."))
    question = labels + b"\0" + struct.pack("!HH", 1, 1)
    return struct.pack("!HHHHHH", query_id, 0x0100, 1, 0, 0, 0) + question


def _dns_answer_ip(response: bytes) -> str:
    assert response[:2] == b"\xca\xfe"
    _, flags, qdcount, ancount, _, _ = struct.unpack("!HHHHHH", response[:12])
    assert flags & 0x8000
    assert flags & 0x000F == 0
    assert qdcount == 1
    assert ancount == 1
    offset = 12
    while response[offset] != 0:
        offset += 1 + response[offset]
    offset += 1 + 4
    _, rr_type, rr_class, _, rdlength = struct.unpack("!HHHIH", response[offset:offset + 12])
    offset += 12
    assert rr_type == 1
    assert rr_class == 1
    assert rdlength == 4
    return ".".join(str(part) for part in response[offset:offset + 4])


def test_mock_server_advertises_all_release_protocol_surfaces() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()

        assert ready["service"] == "capsem-mock-server"
        assert ready["base_url"].startswith("http://127.0.0.1:")
        assert ready["https_base_url"].startswith("https://127.0.0.1:")
        assert ready["dns_udp_addr"].startswith("127.0.0.1:")
        assert ready["dns_tcp_addr"].startswith("127.0.0.1:")
        assert Path(ready["request_log"]).name == "requests.jsonl"

        assert {
            "/tiny",
            "/sse/model",
            "/v1/chat/completions",
            "/v1/responses",
            "/v1/messages",
            "/v1beta/models/gemini-3.5-flash:streamGenerateContent",
            "/v1internal:streamGenerateContent",
            "/api/chat",
            "/oauth/authorize",
            "/oauth/token",
            "/mcp",
            "/ws/echo",
        } <= set(ready["endpoints"])
        assert {
            "fixture.capsem.test",
            "api.openai.com",
            "api.anthropic.com",
            "daily-cloudcode-pa.googleapis.com",
        } <= set(ready["dns_fixtures"])
    finally:
        stop_process(proc)


def test_mock_server_serves_release_protocol_fixtures_from_one_process() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        base_url = ready["base_url"]

        with urlopen(f"{base_url}/tiny", timeout=5) as response:
            assert response.status == 200
            assert response.read() == b"capsem-mock-server:tiny\n"

        with urlopen(f"{base_url}/sse/model", timeout=5) as response:
            sse = response.read().decode()
        assert "event: model.delta" in sse
        assert "event: model.tool_call" in sse

        openai = _post_json(
            f"{base_url}/v1/responses",
            {"model": "mock-local", "input": "write a poem"},
        )
        assert openai["object"] == "response"
        assert openai["output"][0]["type"] == "function_call"

        anthropic = _post_json(
            f"{base_url}/v1/messages",
            {"model": "claude-sonnet-4-6", "messages": [{"role": "user", "content": "hi"}]},
        )
        assert anthropic["type"] == "message"
        assert anthropic["model"] == "claude-sonnet-4-6"

        gemini = _post_json(
            f"{base_url}/v1beta/models/gemini-3.5-flash:streamGenerateContent?alt=sse",
            {"contents": [{"role": "user", "parts": [{"text": "hello"}]}]},
        )
        assert "modelVersion" in gemini["_stream"]

        agy = _post_json(
            f"{base_url}/v1internal:streamGenerateContent?alt=sse",
            {"request": {"contents": [{"role": "user", "parts": [{"text": "hello"}]}]}},
        )
        assert "responseId" in agy["_stream"]

        ollama = _post_json(
            f"{base_url}/api/chat",
            {"model": "gemma4:latest", "messages": [{"role": "user", "content": "hi"}]},
        )
        assert ollama["model"] == "gemma4:latest"
        assert ollama["message"]["role"] == "assistant"

        oauth = _post_json(f"{base_url}/oauth/token", {"code": "capsem-test-code"})
        assert oauth["access_token"].startswith("capsem_test_oauth_access_")

        mcp = _post_json(
            f"{base_url}/mcp",
            {"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
        )
        assert [tool["name"] for tool in mcp["result"]["tools"]] == [
            "fixture_lookup",
            "fetch_http",
            "slow_sleep",
        ]

        host, port_text = ready["dns_udp_addr"].rsplit(":", 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(5)
            sock.sendto(_dns_query("fixture.capsem.test"), (host, int(port_text)))
            response, _ = sock.recvfrom(512)
        assert _dns_answer_ip(response) == "127.0.0.1"
    finally:
        stop_process(proc)
