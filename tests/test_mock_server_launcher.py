from __future__ import annotations

import json
import re
import socket
import ssl
import struct
import threading
import time
from pathlib import Path
from urllib.request import Request, urlopen

from helpers.mock_server import start_mock_server, stop_process


def test_mock_server_launcher_waits_for_busy_address_then_starts() -> None:
    holder = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    holder.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    holder.bind(("127.0.0.1", 0))
    holder.listen(1)
    host, port = holder.getsockname()
    addr = f"{host}:{port}"

    def release_holder() -> None:
        time.sleep(0.3)
        holder.close()

    threading.Thread(target=release_holder, daemon=True).start()
    proc = None
    try:
        proc, ready = start_mock_server(addr=addr, timeout_s=5, retry_interval_s=0.05)
        assert ready["service"] == "capsem-mock-server"
        assert ready["base_url"] == f"http://{addr}"
    finally:
        stop_process(proc)


def test_mock_server_serves_https_fixture() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        assert ready["service"] == "capsem-mock-server"
        assert ready["https_base_url"].startswith("https://127.0.0.1:")
        context = ssl._create_unverified_context()
        with urlopen(f"{ready['https_base_url']}/tiny", context=context, timeout=2) as response:
            assert response.status == 200
            assert response.headers["content-type"] == "text/plain; charset=utf-8"
            assert response.read() == b"capsem-mock-server:tiny\n"
    finally:
        stop_process(proc)


def _dns_query(name: str, qtype: int = 1, query_id: int = 0x1234) -> bytes:
    labels = b"".join(bytes([len(part)]) + part.encode("ascii") for part in name.split("."))
    question = labels + b"\0" + struct.pack("!HH", qtype, 1)
    return struct.pack("!HHHHHH", query_id, 0x0100, 1, 0, 0, 0) + question


def _answer_ip(response: bytes) -> str:
    assert len(response) >= 12
    _, flags, qdcount, ancount, _, _ = struct.unpack("!HHHHHH", response[:12])
    assert flags & 0x8000, "expected DNS response"
    assert flags & 0x000F == 0, f"expected NOERROR, flags={flags:#x}"
    assert qdcount == 1
    assert ancount == 1
    offset = 12
    while response[offset] != 0:
        offset += 1 + response[offset]
    offset += 1 + 4
    name_ptr, rr_type, rr_class, ttl, rdlength = struct.unpack("!HHHIH", response[offset:offset + 12])
    offset += 12
    assert name_ptr == 0xC00C
    assert rr_type == 1
    assert rr_class == 1
    assert ttl == 60
    assert rdlength == 4
    return ".".join(str(part) for part in response[offset:offset + 4])


def test_mock_server_serves_dns_udp_fixture() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        assert ready["service"] == "capsem-mock-server"
        assert ready["dns_udp_addr"].startswith("127.0.0.1:")
        assert ready["dns_tcp_addr"].startswith("127.0.0.1:")

        host, port_text = ready["dns_udp_addr"].rsplit(":", 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(2)
            sock.sendto(_dns_query("fixture.capsem.test"), (host, int(port_text)))
            response, _ = sock.recvfrom(512)

        assert response[:2] == b"\x12\x34"
        assert _answer_ip(response) == "127.0.0.1"
    finally:
        stop_process(proc)


def test_mock_server_serves_dns_tcp_fixture() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        host, port_text = ready["dns_tcp_addr"].rsplit(":", 1)
        query = _dns_query("mcp.capsem.test", query_id=0x4321)
        with socket.create_connection((host, int(port_text)), timeout=2) as sock:
            sock.sendall(struct.pack("!H", len(query)) + query)
            length_bytes = sock.recv(2)
            assert len(length_bytes) == 2
            length = struct.unpack("!H", length_bytes)[0]
            response = sock.recv(length)

        assert response[:2] == b"\x43\x21"
        assert _answer_ip(response) == "127.0.0.1"
    finally:
        stop_process(proc)


def _post_json(url: str, value: object) -> dict:
    request = Request(
        url,
        data=json.dumps(value).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urlopen(request, timeout=2) as response:
        assert response.status == 200
        assert response.headers["content-type"] == "application/json"
        body = json.loads(response.read().decode())
    assert isinstance(body, dict)
    return body


def test_mock_server_replays_ollama_openai_chat_completion_shape() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        base_url = ready["base_url"]
        request_log = Path(ready["request_log"])
        assert request_log.name == "requests.jsonl"

        tool_payload = _post_json(
            f"{base_url}/v1/chat/completions",
            {
                "model": "gemma4:latest",
                "messages": [{"role": "user", "content": "call fixture_lookup"}],
                "tools": [
                    {
                        "type": "function",
                        "function": {
                            "name": "fixture_lookup",
                            "parameters": {
                                "type": "object",
                                "properties": {"query": {"type": "string"}},
                            },
                        },
                    }
                ],
            },
        )
        assert set(tool_payload) == {
            "id",
            "object",
            "created",
            "model",
            "system_fingerprint",
            "choices",
            "usage",
        }
        assert re.fullmatch(r"chatcmpl-\d+", tool_payload["id"])
        assert tool_payload["object"] == "chat.completion"
        assert tool_payload["created"] == 1781444656
        assert tool_payload["model"] == "gemma4:latest"
        assert tool_payload["system_fingerprint"] == "fp_ollama"
        assert tool_payload["usage"] == {
            "prompt_tokens": 66,
            "completion_tokens": 390,
            "total_tokens": 456,
        }
        choice = tool_payload["choices"][0]
        assert choice["index"] == 0
        assert choice["finish_reason"] == "tool_calls"
        message = choice["message"]
        assert message["role"] == "assistant"
        assert message["content"] == ""
        assert isinstance(message["reasoning"], str)
        assert "Ollama-compatible" in message["reasoning"]
        assert len(message["tool_calls"]) == 1
        tool_call = message["tool_calls"][0]
        assert tool_call == {
            "id": "call_fm3e3d2f",
            "index": 0,
            "type": "function",
            "function": {
                "name": "fixture_lookup",
                "arguments": '{"query":"Capsem ironbank poem"}',
            },
        }

        text_payload = _post_json(
            f"{base_url}/v1/chat/completions",
            {
                "model": "gemma4:latest",
                "messages": [{"role": "user", "content": "write poem"}],
            },
        )
        assert "provider" not in text_payload
        assert text_payload["id"] == "chatcmpl-515"
        assert text_payload["created"] == 1781444596
        assert text_payload["system_fingerprint"] == "fp_ollama"
        assert text_payload["choices"][0]["finish_reason"] == "stop"
        assert text_payload["choices"][0]["message"]["content"] == (
            "Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw"
        )
        assert "tool_calls" not in text_payload["choices"][0]["message"]
        assert text_payload["usage"] == {
            "prompt_tokens": 26,
            "completion_tokens": 52,
            "total_tokens": 78,
        }

        records = [json.loads(line) for line in request_log.read_text().splitlines()]
        assert len(records) == 2
        first_record = records[0]
        assert first_record["method"] == "POST"
        assert first_record["path"] == "/v1/chat/completions"
        assert first_record["status"] == 200
        assert first_record["content_type"] == "application/json"
        assert first_record["request_bytes"] == len(first_record["request_body"].encode())
        assert first_record["response_bytes"] == len(first_record["response_body"].encode())
        assert json.loads(first_record["request_body"])["tools"][0]["function"]["name"] == (
            "fixture_lookup"
        )
        assert json.loads(first_record["response_body"]) == tool_payload
    finally:
        stop_process(proc)
