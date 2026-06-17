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


def test_mock_server_head_tiny_matches_get_fixture_headers() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        request = Request(f"{ready['base_url']}/tiny", method="HEAD")
        with urlopen(request, timeout=2) as response:
            assert response.status == 200
            assert response.headers["content-type"] == "text/plain; charset=utf-8"
            assert response.headers["content-length"] == str(len(b"capsem-mock-server:tiny\n"))
            assert response.read() == b""
    finally:
        stop_process(proc)


def test_mock_server_serves_slow_chunks_alias_for_doctor() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        with urlopen(f"{ready['base_url']}/delayed-chunks", timeout=2) as response:
            body = response.read().decode()
            assert response.status == 200
            assert response.headers["content-type"] == "text/plain; charset=utf-8"
            assert "chunk-0" in body
            assert "chunk-3" in body
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

        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(2)
            sock.sendto(_dns_query("api.openai.com", query_id=0x5678), (host, int(port_text)))
            response, _ = sock.recvfrom(512)

        assert response[:2] == b"\x56\x78"
        assert _answer_ip(response) == "127.0.0.1"

        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(2)
            sock.sendto(_dns_query("api.anthropic.com", query_id=0x9ABC), (host, int(port_text)))
            response, _ = sock.recvfrom(512)

        assert response[:2] == b"\x9a\xbc"
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


def _post_raw(url: str, value: object) -> str:
    request = Request(
        url,
        data=json.dumps(value).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urlopen(request, timeout=2) as response:
        assert response.status == 200
        assert response.headers["content-type"] == "text/event-stream"
        return response.read().decode()


def _get_json(url: str) -> dict:
    with urlopen(url, timeout=2) as response:
        assert response.status == 200
        assert response.headers["content-type"] == "application/json"
        body = json.loads(response.read().decode())
    assert isinstance(body, dict)
    return body


def _post_chunked_raw(host: str, port: int, path: str, value: object) -> str:
    payload = json.dumps(value).encode()
    first = payload[:17]
    second = payload[17:]
    request = (
        f"POST {path} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        "User-Agent: capsem-test\r\n"
        "Content-Type: application/json\r\n"
        "Transfer-Encoding: chunked\r\n"
        "\r\n"
    ).encode()
    request += f"{len(first):x}\r\n".encode() + first + b"\r\n"
    request += f"{len(second):x}\r\n".encode() + second + b"\r\n0\r\n\r\n"
    with socket.create_connection((host, port), timeout=2) as sock:
        sock.sendall(request)
        sock.shutdown(socket.SHUT_WR)
        response = b""
        while True:
            chunk = sock.recv(65536)
            if not chunk:
                break
            response += chunk
    header, _, body = response.partition(b"\r\n\r\n")
    assert b" 200 " in header, header.decode(errors="replace")
    return body.decode()


def test_mock_server_serves_ollama_launcher_probe_endpoints() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        base_url = ready["base_url"]

        head_request = Request(f"{base_url}/", method="HEAD")
        with urlopen(head_request, timeout=2) as response:
            assert response.status == 200
            assert response.read() == b""

        tags = _get_json(f"{base_url}/api/tags")
        assert tags["models"][0]["name"] == "gemma4:latest"
        assert tags["models"][0]["details"]["family"] == "gemma"

        show = _post_json(f"{base_url}/api/show", {"model": "gemma4:latest"})
        assert show["modelfile"] == "FROM gemma4:latest"
        assert show["details"]["parameter_size"] == "7B"
    finally:
        stop_process(proc)


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


def test_mock_server_replays_baked_doctor_openai_smoke_as_tool_call() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        payload = _post_json(
            f"{ready['base_url']}/v1/chat/completions",
            {
                "model": "mock-local",
                "messages": [{"role": "user", "content": "hello"}],
            },
        )

        choice = payload["choices"][0]
        assert choice["finish_reason"] == "tool_calls"
        message = choice["message"]
        assert message["content"] == ""
        assert message["tool_calls"][0]["function"]["name"] == "fixture_lookup"
        assert message["tool_calls"][0]["function"]["arguments"] == (
            '{"query":"Capsem ironbank poem"}'
        )
    finally:
        stop_process(proc)


def test_mock_server_replays_streaming_anthropic_tool_use_shape() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        base_url = ready["base_url"]
        target = "/root/claude-stream-tool-0123456789abcdef0123456789abcdef.txt"
        token = "0123456789abcdef0123456789abcdef"
        body = {
            "model": "gemma4:latest",
            "stream": True,
            "messages": [
                {"role": "user", "content": f"Write uuid4 hex value {token} to {target}."},
                {
                    "role": "system",
                    "content": "Documentation mentions tool_result but this is not a result block.",
                },
            ],
            "tools": [
                {
                    "name": "Bash",
                    "description": "run a command",
                    "input_schema": {
                        "type": "object",
                        "properties": {"command": {"type": "string"}},
                    },
                }
            ],
        }
        stream = _post_raw(f"{base_url}/v1/messages?beta=true", body)

        assert "event: message_start" in stream
        assert "event: content_block_start" in stream
        assert "event: content_block_delta" in stream
        assert "event: message_delta" in stream
        assert "event: message_stop" in stream
        assert '"type":"tool_use"' in stream
        assert '"name":"Bash"' in stream
        assert '"type":"input_json_delta"' in stream
        assert "printf" in stream
        assert token in stream
        assert target in stream
        assert '"stop_reason":"tool_use"' in stream
    finally:
        stop_process(proc)


def test_mock_server_replays_streaming_anthropic_final_shape() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        base_url = ready["base_url"]
        token = "fedcba9876543210fedcba9876543210"
        body = {
            "model": "gemma4:latest",
            "stream": True,
            "messages": [
                {"role": "user", "content": f"Write uuid4 hex value {token} to /root/out.txt."},
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "toolu_capsem_write_poem",
                            "content": "Process exited with code 0",
                        }
                    ],
                },
            ],
            "tools": [{"name": "Bash"}],
        }
        stream = _post_raw(f"{base_url}/v1/messages?beta=true", body)

        assert "event: message_start" in stream
        assert '"type":"thinking"' in stream
        assert '"type":"thinking_delta"' in stream
        assert '"thinking":"ledger reasoning"' in stream
        assert '"type":"text_delta"' in stream
        assert token in stream
        assert '"stop_reason":"end_turn"' in stream
        assert "tool_use" not in stream
    finally:
        stop_process(proc)


def test_mock_server_replays_recorded_agy_code_assist_experiments() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        payload = _post_json(f"{ready['base_url']}/v1internal:listExperiments", {})

        flags = payload["flags"]
        assert len(payload["experimentIds"]) == 68
        assert len(flags) == 250
        assert len(json.dumps(payload, separators=(",", ":")).encode()) > 20_000
        assert {
            "GcliConfigPayload__config_payload",
            "GcliConfig__cli_max_attempts",
            "CliComplexityBasedRouting__enabled",
            "allow-always-config",
            "enable-owl-slash-command",
            "enable-state-accumulator",
        }.issubset({flag["name"] for flag in flags})
        config_payload = next(
            flag["stringValue"]
            for flag in flags
            if flag["name"] == "GcliConfigPayload__config_payload"
        )
        assert config_payload == ""
    finally:
        stop_process(proc)


def test_mock_server_replays_recorded_agy_available_models() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        payload = _post_json(
            f"{ready['base_url']}/v1internal:fetchAvailableModels",
            {"project": "capsem-mock-project"},
        )

        models = payload["models"]
        assert len(models) == 19
        assert payload["defaultAgentModelId"] == "gemini-3.5-flash-low"
        assert payload["defaultAgentModelId"] in models
        assert payload["agentModelSorts"][0]["displayName"] == "Recommended"
        assert (
            payload["agentModelSorts"][0]["groups"][0]["modelIds"][0]
            == payload["defaultAgentModelId"]
        )
        assert "gemini-3-flash-agent" in payload["tieredModelIds"]["flash"]
        referenced_model_ids = {
            payload["defaultAgentModelId"],
            *payload["commandModelIds"],
            *payload["mqueryModelIds"],
            *payload["imageGenerationModelIds"],
            *payload["webSearchModelIds"],
            *payload["tabModelIds"],
            *payload["commitMessageModelIds"],
        }
        for group in payload["agentModelSorts"]:
            for bucket in group["groups"]:
                referenced_model_ids.update(bucket["modelIds"])
        for ids in payload["tieredModelIds"].values():
            referenced_model_ids.update(ids)
        assert referenced_model_ids <= set(models)
        model_enums = {model["model"] for model in models.values()}
        checkpoint_enums = set()
        for model in models.values():
            experiments = model.get("modelExperiments", {}).get("experiments", {})
            for experiment in experiments.values():
                value = experiment.get("stringValue")
                if value:
                    checkpoint_enums.update(
                        re.findall(r'"checkpoint_model"\s*:\s*"(MODEL_[A-Z0-9_]+)"', value)
                    )
        assert checkpoint_enums <= model_enums
        assert models["gemini-3.5-flash-low"]["displayName"] == "Gemini 3.5 Flash (Medium)"
        assert models["gemini-3.5-flash-low"]["model"] == "MODEL_PLACEHOLDER_M20"
        assert models["gemini-3.5-flash-low"]["modelProvider"] == "MODEL_PROVIDER_GOOGLE"
        assert models["claude-sonnet-4-6"]["modelProvider"] == "MODEL_PROVIDER_ANTHROPIC"
        assert all(model["quotaInfo"]["remainingFraction"] == 1 for model in models.values())
    finally:
        stop_process(proc)


def test_mock_server_replays_agy_code_assist_stream_envelope() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        base_url = ready["base_url"]
        token = "0123456789abcdef0123456789abcdef"
        target = f"/root/agy-cli-{token}.txt"

        stream = _post_raw(
            f"{base_url}/v1internal:streamGenerateContent?alt=sse",
            {
                "request": {
                    "contents": [
                        {
                            "role": "user",
                            "parts": [
                                {"text": f"Write uuid4 hex value {token} to {target}."}
                            ],
                        }
                    ]
                }
            },
        )

        chunks = [
            json.loads(chunk.removeprefix("data: ").strip())
            for chunk in stream.split("\n\n")
            if chunk.strip()
        ]
        assert len(chunks) == 2
        assert all(set(chunk) == {"response", "traceId", "metadata"} for chunk in chunks)
        first_response = chunks[0]["response"]
        assert set(first_response) == {
            "candidates",
            "usageMetadata",
            "modelVersion",
            "responseId",
        }
        first_candidate = first_response["candidates"][0]
        first_part = first_candidate["content"]["parts"][0]
        function_call = first_part["functionCall"]
        assert function_call["name"] == "run_command"
        assert function_call["id"] == "call_0123456789ab"
        assert function_call["args"]["Cwd"] == "/root"
        assert function_call["args"]["WaitMsBeforeAsync"] == 1000
        assert function_call["args"]["CommandLine"] == (
            "printf '%s\\n' 0123456789abcdef0123456789abcdef "
            "> /root/agy-cli-0123456789abcdef0123456789abcdef.txt"
        )
        assert first_candidate.get("finishReason") is None
        assert first_response["usageMetadata"]["thoughtsTokenCount"] > 0

        final_candidate = chunks[1]["response"]["candidates"][0]
        assert final_candidate["finishReason"] == "STOP"
        assert final_candidate["content"]["parts"] == [{"text": ""}]
        assert chunks[1]["response"]["responseId"] == first_response["responseId"]
    finally:
        stop_process(proc)


def test_mock_server_reads_agy_chunked_code_assist_body() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        parsed = re.fullmatch(r"http://([^:]+):(\d+)", ready["base_url"])
        assert parsed is not None
        host, port_text = parsed.groups()
        token = "abcdefabcdefabcdefabcdefabcdefab"
        target = f"/root/agy-cli-{token}.txt"

        stream = _post_chunked_raw(
            host,
            int(port_text),
            "/v1internal:streamGenerateContent?alt=sse",
            {
                "request": {
                    "contents": [
                        {
                            "role": "user",
                            "parts": [
                                {"text": f"Write uuid4 hex value {token} to {target}."}
                            ],
                        }
                    ]
                }
            },
        )

        assert target in stream
        assert token in stream
        assert "/root/agy-output.txt" not in stream
    finally:
        stop_process(proc)


def test_mock_server_replays_agy_checkpoint_without_duplicate_tool_call() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        base_url = ready["base_url"]
        token = "11111111111111111111111111111111"
        target = f"/root/agy-cli-{token}.txt"

        stream = _post_raw(
            f"{base_url}/v1internal:streamGenerateContent?alt=sse",
            {
                "requestType": "checkpoint",
                "model": "gemini-3.1-flash-lite",
                "request": {
                    "contents": [
                        {
                            "role": "user",
                            "parts": [
                                {"text": f"Write uuid4 hex value {token} to {target}."}
                            ],
                        }
                    ],
                    "systemInstruction": {
                        "role": "user",
                        "parts": [
                            {
                                "text": "Generate a short conversation title (3-5 words, title-cased, no prefix) describing the USER's intent."
                            }
                        ],
                    },
                },
            },
        )

        chunks = [
            json.loads(chunk.removeprefix("data: ").strip())
            for chunk in stream.split("\n\n")
            if chunk.strip()
        ]
        assert len(chunks) == 1
        response = chunks[0]["response"]
        assert response["modelVersion"] == "gemini-3.1-flash-lite"
        assert "usageMetadata" not in response
        candidate = response["candidates"][0]
        assert candidate["finishReason"] == "STOP"
        assert candidate["content"]["parts"] == [{"text": "Write Proof"}]
        assert "functionCall" not in json.dumps(response)
        assert target not in stream
        assert token not in stream
    finally:
        stop_process(proc)


def test_mock_server_replays_gemini_api_stream_without_code_assist_envelope() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        base_url = ready["base_url"]
        token = "22222222222222222222222222222222"
        target = f"/root/gemini-api-{token}.txt"

        stream = _post_raw(
            f"{base_url}/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
            {
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            {"text": f"Write uuid4 hex value {token} to {target}."}
                        ],
                    }
                ],
                "tools": [
                    {
                        "functionDeclarations": [
                            {
                                "name": "write_to_file",
                                "parameters": {
                                    "type": "object",
                                    "properties": {
                                        "TargetFile": {"type": "string"},
                                        "Content": {"type": "string"},
                                    },
                                    "required": ["TargetFile", "Content"],
                                },
                            }
                        ]
                    }
                ],
            },
        )

        chunks = [
            json.loads(chunk.removeprefix("data: ").strip())
            for chunk in stream.split("\n\n")
            if chunk.strip()
        ]
        assert len(chunks) == 1
        assert "response" not in chunks[0]
        assert chunks[0]["modelVersion"] == "gemini-2.5-flash"
        candidate = chunks[0]["candidates"][0]
        function_call = candidate["content"]["parts"][0]["functionCall"]
        assert function_call["name"] == "write_to_file"
        assert set(function_call["args"]) == {"TargetFile", "Content"}
        assert function_call["args"]["TargetFile"] == target
        assert function_call["args"]["Content"] == token + "\n"
        assert "run_command" not in stream
    finally:
        stop_process(proc)


def test_mock_server_replays_recorded_agy_code_assist_setup() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        base_url = ready["base_url"]

        setup = _post_json(
            f"{base_url}/v1internal:loadCodeAssist",
            {"metadata": {"ideType": "ANTIGRAVITY"}},
        )
        assert setup["currentTier"]["id"] == "free-tier"
        assert setup["cloudaicompanionProject"] == "capsem-mock-project"
        assert len(setup["allowedTiers"]) == 2
        assert len(json.dumps(setup, separators=(",", ":")).encode()) > 3_000

        quota = _post_json(
            f"{base_url}/v1internal:retrieveUserQuotaSummary",
            {"project": "capsem-mock-project"},
        )
        assert {group["displayName"] for group in quota["groups"]} == {
            "Gemini Models",
            "Claude and GPT models",
        }
        assert all(
            bucket["remainingFraction"] == 1
            for group in quota["groups"]
            for bucket in group["buckets"]
        )

        user_info = _post_json(
            f"{base_url}/v1internal:fetchUserInfo",
            {"project": "capsem-mock-project"},
        )
        assert user_info["regionCode"] == "US"
        assert user_info["userSettings"]["telemetryEnabled"] is False
        assert "cachedCascadeModelConfigs" not in user_info["userSettings"]
        assert "userStatus" not in user_info
    finally:
        stop_process(proc)


def test_mock_server_replays_agy_playlog_empty_ack() -> None:
    proc = None
    try:
        proc, ready = start_mock_server()
        request = Request(
            f"{ready['base_url']}/log",
            data=b"\x0a\x04test",
            headers={"content-type": "application/x-protobuf"},
            method="POST",
        )
        with urlopen(request, timeout=5) as response:
            body = response.read()
            content_type = response.headers.get("content-type", "")

        assert response.status == 200
        assert body == b""
        assert "text/plain" in content_type
    finally:
        stop_process(proc)
