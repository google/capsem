#!/usr/bin/env python3
"""Capsem's deterministic local mock server runtime."""

from __future__ import annotations

import argparse
import base64
import gzip
import hashlib
import json
import re
import shlex
import socketserver
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse


TINY_BODY = b"capsem-mock-server:tiny\n"
EXPECTED_POEM = "Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw"
OLLAMA_OPENAI_TOOL_CALL_ID = "call_fm3e3d2f"
OLLAMA_OPENAI_TOOL_ARGUMENTS = '{"query":"Capsem ironbank poem"}'
CODEX_RESPONSES_TOOL_CALL_ID = "call_codex_write_poem"
CODEX_RESPONSES_TOOL_ITEM_ID = "fc_codex_write_poem"
CODEX_RESPONSES_TOOL_NAME = "exec_command"
ANTHROPIC_TOOL_CALL_ID = "toolu_capsem_write_poem"
OLLAMA_TOOL_CALL_ID = "ollama_capsem_write_poem"
HTML_ABOUT = """<!doctype html>
<html>
  <head><title>Capsem Mock Server About</title></head>
  <body>
    <div id="about">
      <p>Capsem mock server about page for local MCP fetch tests.</p>
      <p>Google, Anthropic, and OpenAI appear here as fixture text only.</p>
      <a href="https://example.invalid/local">Local fixture link</a>
    </div>
  </body>
</html>
"""
ENDPOINTS = [
    "/tiny",
    "/html/about",
    "/html/large",
    "/bytes/{size}",
    "/gzip/{size}",
    "/sse/model",
    "/model/response",
    "/model/shape",
    "/model/no-tool-call",
    "/v1beta/models/gemini-2.5-flash:streamGenerateContent",
    "/v1/chat/completions",
    "/v1/responses",
    "/v1/messages",
    "/api/chat",
    "/api/show",
    "/api/tags",
    "/oauth/authorize",
    "/oauth/token",
    "/mcp",
    "/slow-chunks",
    "/credential/response",
    "/echo",
    "/deny-target",
    "/ws/echo",
    "/ws/ping",
    "/ws/close",
]
DNS_FIXTURES = {
    "fixture.capsem.test": "127.0.0.1",
    "model.capsem.test": "127.0.0.1",
    "mcp.capsem.test": "127.0.0.1",
}
REQUEST_LOG_PATH: Path | None = None
REQUEST_LOG_LOCK = threading.Lock()


def _deterministic_bytes(size: str) -> bytes:
    lengths = {"10kb": 10 * 1024, "1mb": 1024 * 1024, "10mb": 10 * 1024 * 1024}
    try:
        length = lengths[size.lower()]
    except KeyError as exc:
        raise ValueError(f"unsupported size '{size}'") from exc
    return bytes(ord("a") + (idx % 26) for idx in range(length))


def _model_payload(
    model: str = "mock-local",
    *,
    include_tool_call: bool = True,
    ollama_tool_shape: bool = False,
) -> dict:
    tool_call_content = "" if ollama_tool_shape else EXPECTED_POEM
    message = {
        "role": "assistant",
        "content": tool_call_content if include_tool_call else EXPECTED_POEM,
        "reasoning": "Deterministic local Ollama-compatible fixture reasoning.",
    }
    if include_tool_call:
        message["tool_calls"] = [
            {
                "id": OLLAMA_OPENAI_TOOL_CALL_ID,
                "index": 0,
                "type": "function",
                "function": {
                    "name": "fixture_lookup",
                    "arguments": OLLAMA_OPENAI_TOOL_ARGUMENTS,
                },
            }
        ]
    usage = (
        {"prompt_tokens": 66, "completion_tokens": 390, "total_tokens": 456}
        if include_tool_call
        else {"prompt_tokens": 26, "completion_tokens": 52, "total_tokens": 78}
    )
    return {
        "id": "chatcmpl-601" if include_tool_call else "chatcmpl-515",
        "object": "chat.completion",
        "created": 1781444656 if include_tool_call else 1781444596,
        "model": model,
        "system_fingerprint": "fp_ollama",
        "choices": [
            {
                "index": 0,
                "message": message,
                "finish_reason": "tool_calls" if include_tool_call else "stop",
            }
        ],
        "usage": usage,
    }


def _responses_payload(model: str = "mock-local") -> dict:
    return _responses_payload_for_output(model, EXPECTED_POEM)


def _responses_payload_for_output(model: str = "mock-local", output_text: str = EXPECTED_POEM) -> dict:
    return {
        "id": "resp_ironbank_01",
        "object": "response",
        "created_at": 1781205836,
        "status": "completed",
        "model": model,
        "output": [
            {
                "id": "msg_ironbank_01",
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": output_text,
                        "annotations": [],
                    }
                ],
            }
        ],
        "output_text": output_text,
        "usage": {
            "input_tokens": 7,
            "output_tokens": 5,
            "total_tokens": 12,
            "output_tokens_details": {"reasoning_tokens": 2},
        },
    }


def _codex_responses_write_target(payload: dict) -> tuple[str, str]:
    body = json.dumps(payload, separators=(",", ":"))
    token_match = re.search(r"uuid4 hex value ([0-9a-f]{32})", body)
    path_match = re.search(r"(/root/[a-z0-9_-]+-[0-9a-f]{32}\.txt)", body)
    token = token_match.group(1) if token_match else EXPECTED_POEM
    path = path_match.group(1) if path_match else "/root/codex-cli-output.txt"
    return token, path


def _responses_call_id_for_payload(payload: dict) -> str:
    token, _ = _codex_responses_write_target(payload)
    if re.fullmatch(r"[0-9a-f]{32}", token):
        return f"call_{token[:12]}"
    return CODEX_RESPONSES_TOOL_CALL_ID


def _responses_item_id_for_payload(payload: dict) -> str:
    token, _ = _codex_responses_write_target(payload)
    if re.fullmatch(r"[0-9a-f]{32}", token):
        return f"fc_{token[:12]}"
    return CODEX_RESPONSES_TOOL_ITEM_ID


def _generic_write_target(payload: dict, default_prefix: str) -> tuple[str, str]:
    body = json.dumps(payload, separators=(",", ":"))
    token_match = re.search(r"uuid4 hex value ([0-9a-f]{32})", body)
    path_match = re.search(r"(/root/[a-z0-9_-]+-[0-9a-f]{32}\.txt)", body)
    token = token_match.group(1) if token_match else EXPECTED_POEM
    path = path_match.group(1) if path_match else f"/root/{default_prefix}-output.txt"
    return token, path


def _shell_write_command(token: str, path: str) -> str:
    return f"printf '%s\\n' {shlex.quote(token)} > {shlex.quote(path)}"


def _codex_responses_tool_arguments(payload: dict) -> str:
    token, path = _codex_responses_write_target(payload)
    return json.dumps(
        {
            "cmd": f"printf '%s\\n' {shlex.quote(token)} > {shlex.quote(path)}",
            "yield_time_ms": 1000,
            "max_output_tokens": 2000,
        },
        separators=(",", ":"),
    )


def _responses_tool_call_payload(model: str = "mock-local", payload: dict | None = None) -> dict:
    payload = payload or {}
    call_id = _responses_call_id_for_payload(payload)
    item_id = _responses_item_id_for_payload(payload)
    return {
        "id": "resp_ironbank_tool_01",
        "object": "response",
        "created_at": 1781205836,
        "status": "completed",
        "model": model,
        "output": [
            {
                "id": item_id,
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": CODEX_RESPONSES_TOOL_NAME,
                "arguments": _codex_responses_tool_arguments(payload),
            }
        ],
        "usage": {
            "input_tokens": 31,
            "output_tokens": 17,
            "total_tokens": 48,
        },
    }


def _responses_payload_has_tool_output(payload: dict) -> bool:
    body = json.dumps(payload, separators=(",", ":"))
    return "function_call_output" in body


def _responses_tool_call_stream_body(model: str = "mock-local", payload: dict | None = None) -> bytes:
    payload = payload or {}
    call_id = _responses_call_id_for_payload(payload)
    item_id = _responses_item_id_for_payload(payload)
    response = {
        "id": "resp_ironbank_tool_01",
        "object": "response",
        "created_at": 1781205836,
        "status": "in_progress",
        "model": model,
        "output": [],
    }
    created = {"type": "response.created", "response": response}
    item_started = {
        "type": "response.output_item.added",
        "output_index": 0,
        "item": {
            "id": item_id,
            "type": "function_call",
            "status": "in_progress",
            "call_id": call_id,
            "name": CODEX_RESPONSES_TOOL_NAME,
            "arguments": "",
        },
    }
    arguments_done = {
        "type": "response.function_call_arguments.done",
        "output_index": 0,
        "item_id": item_id,
        "arguments": _codex_responses_tool_arguments(payload),
    }
    item_done = {
        "type": "response.output_item.done",
        "output_index": 0,
        "item": _responses_tool_call_payload(model, payload)["output"][0],
    }
    completed = {"type": "response.completed", "response": _responses_tool_call_payload(model, payload)}
    arguments = _codex_responses_tool_arguments(payload)
    return (
        f"event: response.created\ndata: {json.dumps(created, separators=(',', ':'))}\n\n"
        f"event: response.output_item.added\ndata: {json.dumps(item_started, separators=(',', ':'))}\n\n"
        f"event: response.function_call_arguments.delta\ndata: "
        f"{json.dumps({'type': 'response.function_call_arguments.delta', 'output_index': 0, 'item_id': item_id, 'delta': arguments}, separators=(',', ':'))}\n\n"
        f"event: response.function_call_arguments.done\ndata: {json.dumps(arguments_done, separators=(',', ':'))}\n\n"
        f"event: response.output_item.done\ndata: {json.dumps(item_done, separators=(',', ':'))}\n\n"
        f"event: response.completed\ndata: {json.dumps(completed, separators=(',', ':'))}\n\n"
    ).encode()


def _responses_stream_body(model: str = "mock-local", payload: dict | None = None) -> bytes:
    output_text, _ = _codex_responses_write_target(payload or {})
    reasoning_text = "ledger reasoning"
    response = {
        "id": "resp_ironbank_01",
        "object": "response",
        "created_at": 1781205836,
        "status": "in_progress",
        "model": model,
        "output": [],
    }
    created = {"type": "response.created", "response": response}
    completed = {
        "type": "response.completed",
        "response": _responses_payload_for_output(model, output_text),
    }
    message_item = completed["response"]["output"][0]
    content_part = message_item["content"][0]
    return (
        f"event: response.created\ndata: {json.dumps(created, separators=(',', ':'))}\n\n"
        'event: response.output_item.added\n'
        'data: {"type":"response.output_item.added","output_index":0,'
        '"item":{"id":"msg_ironbank_01","type":"message","status":"in_progress",'
        '"role":"assistant","content":[]}}\n\n'
        'event: response.content_part.added\n'
        'data: {"type":"response.content_part.added","item_id":"msg_ironbank_01",'
        '"output_index":0,"content_index":0,'
        '"part":{"type":"output_text","text":"","annotations":[]}}\n\n'
        f"event: response.reasoning_summary_text.delta\ndata: "
        f"{json.dumps({'type': 'response.reasoning_summary_text.delta', 'item_id': 'msg_ironbank_01', 'output_index': 0, 'summary_index': 0, 'delta': reasoning_text}, separators=(',', ':'))}\n\n"
        f"event: response.output_text.delta\ndata: "
        f"{json.dumps({'type': 'response.output_text.delta', 'item_id': 'msg_ironbank_01', 'output_index': 0, 'content_index': 0, 'delta': output_text}, separators=(',', ':'))}\n\n"
        f"event: response.output_text.done\ndata: "
        f"{json.dumps({'type': 'response.output_text.done', 'item_id': 'msg_ironbank_01', 'output_index': 0, 'content_index': 0, 'text': output_text}, separators=(',', ':'))}\n\n"
        f"event: response.content_part.done\ndata: "
        f"{json.dumps({'type': 'response.content_part.done', 'item_id': 'msg_ironbank_01', 'output_index': 0, 'content_index': 0, 'part': content_part}, separators=(',', ':'))}\n\n"
        f"event: response.output_item.done\ndata: "
        f"{json.dumps({'type': 'response.output_item.done', 'output_index': 0, 'item': message_item}, separators=(',', ':'))}\n\n"
        f"event: response.completed\ndata: {json.dumps(completed, separators=(',', ':'))}\n\n"
    ).encode()


def _google_stream_body() -> bytes:
    return (
        'data: {"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"}}],'
        '"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":1},'
        '"modelVersion":"gemini-2.5-flash"}\n\n'
        'data: {"candidates":[{"content":{"parts":[{"text":" world!"}],"role":"model"}}],'
        '"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":3}}\n\n'
        'data: {"candidates":[{"content":{"parts":[{"text":""}],"role":"model"},'
        '"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,'
        '"candidatesTokenCount":3,"totalTokenCount":8}}\n\n'
    ).encode()


def _anthropic_stream_body() -> bytes:
    return (
        'event: message_start\n'
        'data: {"type":"message_start","message":{"id":"msg_ironbank_01",'
        '"model":"claude-sonnet-4-20250514",'
        '"usage":{"input_tokens":25,"output_tokens":1}}}\n\n'
        'event: content_block_start\n'
        'data: {"type":"content_block_start","index":0,'
        '"content_block":{"type":"text","text":""}}\n\n'
        'event: content_block_delta\n'
        'data: {"type":"content_block_delta","index":0,'
        '"delta":{"type":"text_delta","text":"Hello"}}\n\n'
        'event: content_block_delta\n'
        'data: {"type":"content_block_delta","index":0,'
        '"delta":{"type":"text_delta","text":" world!"}}\n\n'
        'event: content_block_stop\n'
        'data: {"type":"content_block_stop","index":0}\n\n'
        'event: message_delta\n'
        'data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},'
        '"usage":{"output_tokens":5}}\n\n'
        'event: message_stop\n'
        'data: {"type":"message_stop"}\n\n'
    ).encode()


def _anthropic_message_payload(model: str = "claude-sonnet-4-20250514") -> dict:
    return {
        "id": "msg_ironbank_01",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [{"type": "text", "text": EXPECTED_POEM}],
        "stop_reason": "end_turn",
        "stop_sequence": None,
        "usage": {"input_tokens": 25, "output_tokens": 5},
    }


def _anthropic_has_tool_result(payload: dict) -> bool:
    return "tool_result" in json.dumps(payload, separators=(",", ":"))


def _anthropic_tool_name(payload: dict) -> str:
    tools = payload.get("tools")
    if isinstance(tools, list):
        names = [tool.get("name") for tool in tools if isinstance(tool, dict)]
        for preferred in ("exec_command", "Bash", "bash"):
            if preferred in names:
                return preferred
        for name in names:
            if isinstance(name, str) and name:
                return name
    return "exec_command"


def _anthropic_tool_input(name: str, token: str, path: str) -> dict:
    command = _shell_write_command(token, path)
    if name == "Bash":
        return {"command": command, "description": "write ironbank token"}
    if name in {"write_file", "Write"}:
        return {"file_path": path, "content": f"{token}\n"}
    return {"cmd": command, "yield_time_ms": 1000, "max_output_tokens": 2000}


def _anthropic_tool_use_payload(
    model: str = "claude-sonnet-4-20250514",
    payload: dict | None = None,
) -> dict:
    payload = payload or {}
    token, path = _generic_write_target(payload, "claude")
    tool_name = _anthropic_tool_name(payload)
    return {
        "id": "msg_ironbank_tool_01",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [
            {
                "type": "tool_use",
                "id": ANTHROPIC_TOOL_CALL_ID,
                "name": tool_name,
                "input": _anthropic_tool_input(tool_name, token, path),
            }
        ],
        "stop_reason": "tool_use",
        "stop_sequence": None,
        "usage": {"input_tokens": 31, "output_tokens": 17},
    }


def _anthropic_final_payload(
    model: str = "claude-sonnet-4-20250514",
    payload: dict | None = None,
) -> dict:
    payload = payload or {}
    token, _ = _generic_write_target(payload, "claude")
    return {
        "id": "msg_ironbank_final_01",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [
            {"type": "thinking", "thinking": "ledger reasoning"},
            {"type": "text", "text": token},
        ],
        "stop_reason": "end_turn",
        "stop_sequence": None,
        "usage": {"input_tokens": 7, "output_tokens": 5},
    }


def _ollama_chat_payload(model: str = "gemma4:latest") -> dict:
    return {
        "model": model,
        "created_at": "2026-06-13T00:00:00Z",
        "message": {"role": "assistant", "content": EXPECTED_POEM},
        "done": True,
        "prompt_eval_count": 7,
        "eval_count": 5,
    }


def _ollama_has_tool_result(payload: dict) -> bool:
    return "tool" in json.dumps(payload, separators=(",", ":")).lower() and (
        "result" in json.dumps(payload, separators=(",", ":")).lower()
        or "output" in json.dumps(payload, separators=(",", ":")).lower()
    )


def _ollama_chat_tool_payload(model: str = "gemma4:latest", payload: dict | None = None) -> dict:
    payload = payload or {}
    token, path = _generic_write_target(payload, "agy")
    return {
        "model": model,
        "created_at": "2026-06-13T00:00:00Z",
        "message": {
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {
                    "function": {
                        "name": "exec_command",
                        "arguments": {
                            "cmd": _shell_write_command(token, path),
                            "yield_time_ms": 1000,
                            "max_output_tokens": 2000,
                        },
                    }
                }
            ],
        },
        "done": True,
        "prompt_eval_count": 31,
        "eval_count": 17,
    }


def _ollama_chat_final_payload(model: str = "gemma4:latest", payload: dict | None = None) -> dict:
    payload = payload or {}
    token, _ = _generic_write_target(payload, "agy")
    return {
        "model": model,
        "created_at": "2026-06-13T00:00:00Z",
        "message": {
            "role": "assistant",
            "content": token,
            "thinking": "ledger reasoning",
        },
        "done": True,
        "prompt_eval_count": 7,
        "eval_count": 5,
    }


class MockHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "capsem-mock-server/1.0"

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def _body(self) -> bytes:
        length = int(self.headers.get("content-length") or "0")
        body = self.rfile.read(length) if length else b""
        self._capsem_request_body = body
        return body

    def _json_body(self) -> dict:
        body = self._body()
        if not body:
            return {}
        try:
            value = json.loads(body)
        except json.JSONDecodeError:
            return {}
        return value if isinstance(value, dict) else {}

    def _send(self, status: int, body: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        self._record_request(status, content_type, body)

    def _send_json(self, value: object, status: int = HTTPStatus.OK) -> None:
        body = json.dumps(value, separators=(",", ":")).encode()
        self._send(status, body, "application/json")

    def _record_request(self, status: int, content_type: str, response_body: bytes) -> None:
        if REQUEST_LOG_PATH is None:
            return
        request_body = getattr(self, "_capsem_request_body", b"")
        record = {
            "method": self.command,
            "path": urlparse(self.path).path,
            "query": urlparse(self.path).query,
            "headers": {key.lower(): value for key, value in self.headers.items()},
            "status": int(status),
            "content_type": content_type,
            "request_body": request_body.decode("utf-8", errors="replace"),
            "response_body": response_body.decode("utf-8", errors="replace"),
            "request_bytes": len(request_body),
            "response_bytes": len(response_body),
        }
        line = json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n"
        with REQUEST_LOG_LOCK:
            REQUEST_LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
            with REQUEST_LOG_PATH.open("a", encoding="utf-8") as handle:
                handle.write(line)

    def do_HEAD(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path
        status = HTTPStatus.OK if path == "/" else HTTPStatus.NOT_FOUND
        self.send_response(status)
        self.send_header("content-length", "0")
        self.end_headers()
        self._record_request(status, "application/octet-stream", b"")

    def do_GET(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path
        if self.headers.get("upgrade", "").lower() == "websocket":
            self._websocket(path)
            return
        if path == "/tiny":
            self._send(HTTPStatus.OK, TINY_BODY, "text/plain; charset=utf-8")
        elif path == "/html/about":
            self._send(HTTPStatus.OK, HTML_ABOUT.encode(), "text/html; charset=utf-8")
        elif path == "/html/large":
            body = "<!doctype html><html><body><main>\n"
            for idx in range(80):
                body += (
                    f"<p>Capsem local pagination fixture paragraph {idx}: "
                    "mock server content for MCP fetch tests.</p>\n"
                )
            body += "</main></body></html>\n"
            self._send(HTTPStatus.OK, body.encode(), "text/html; charset=utf-8")
        elif path.startswith("/bytes/"):
            self._bytes(path.removeprefix("/bytes/"), gzip_body=False)
        elif path.startswith("/gzip/"):
            self._bytes(path.removeprefix("/gzip/"), gzip_body=True)
        elif path == "/sse/model":
            body = (
                'event: model.delta\ndata: {"provider":"mock","model":"mock-local",'
                '"content":"hello"}\n\n'
                'event: model.tool_call\ndata: {"id":"tool_0001","name":"fixture_lookup",'
                '"arguments":{"query":"capsem"}}\n\n'
                'event: model.done\ndata: {"finish_reason":"stop"}\n\n'
            ).encode()
            self._send(HTTPStatus.OK, body, "text/event-stream")
        elif path == "/model/response":
            self._send_json(_model_payload())
        elif path == "/oauth/authorize":
            self._send_json(
                {
                    "kind": "synthetic_oauth_authorization_fixture",
                    "authorization_code": "capsem_test_oauth_code_0123456789abcdef",
                    "redirect_uri": "https://capsem.invalid/oauth/callback",
                    "state": "capsem-fixture-state",
                    "scope": "openid profile email offline_access",
                }
            )
        elif path == "/slow-chunks":
            self.send_response(HTTPStatus.OK)
            self.send_header("content-type", "text/plain; charset=utf-8")
            self.send_header("connection", "close")
            self.end_headers()
            for idx in range(4):
                time.sleep(0.01)
                self.wfile.write(f"chunk-{idx}\n".encode())
                self.wfile.flush()
            self.close_connection = True
        elif path == "/credential/response":
            self._send_json(
                {
                    "kind": "synthetic_credential_fixture",
                    "api_key": "sk-capsem_test_api_key_0123456789abcdef",
                    "oauth": {
                        "access_token": "capsem_test_oauth_access_0123456789abcdef",
                        "refresh_token": "capsem_test_oauth_refresh_0123456789abcdef",
                        "expires_in": 3600,
                    },
                }
            )
        elif path == "/api/tags":
            self._send_json(
                {
                    "models": [
                        {
                            "name": "gemma4:latest",
                            "model": "gemma4:latest",
                            "modified_at": "2026-06-13T00:00:00Z",
                            "size": 123456,
                            "digest": "sha256:capsem-mock-gemma4",
                            "details": {
                                "format": "gguf",
                                "family": "gemma",
                                "parameter_size": "7B",
                                "quantization_level": "Q4_0",
                            },
                        }
                    ]
                }
            )
        elif path == "/deny-target":
            self._send(HTTPStatus.OK, b"capsem-mock-server:deny-target\n", "text/plain")
        else:
            self._send_json({"error": "not found"}, HTTPStatus.NOT_FOUND)

    def do_POST(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path
        if path == "/v1/chat/completions":
            payload = self._json_body()
            model = payload.get("model") if isinstance(payload.get("model"), str) else "mock-local"
            include_tool_call = bool(payload.get("tools"))
            self._send_json(
                _model_payload(
                    model,
                    include_tool_call=include_tool_call,
                    ollama_tool_shape=include_tool_call,
                )
            )
        elif path == "/v1/responses":
            payload = self._json_body()
            model = payload.get("model") if isinstance(payload.get("model"), str) else "mock-local"
            has_tool_output = _responses_payload_has_tool_output(payload)
            if payload.get("stream") is True:
                body = (
                    _responses_stream_body(model, payload)
                    if has_tool_output
                    else _responses_tool_call_stream_body(model, payload)
                )
                self._send(HTTPStatus.OK, body, "text/event-stream")
            else:
                self._send_json(
                    _responses_payload_for_output(model, _codex_responses_write_target(payload)[0])
                    if has_tool_output
                    else _responses_tool_call_payload(model, payload)
                )
        elif path == "/model/shape":
            payload = self._json_body()
            model = payload.get("model") if isinstance(payload.get("model"), str) else "mock-local"
            self._send_json(_model_payload(model))
        elif path == "/model/no-tool-call":
            payload = self._json_body()
            model = payload.get("model") if isinstance(payload.get("model"), str) else "mock-local"
            self._send_json(_model_payload(model, include_tool_call=False))
        elif path.endswith(":streamGenerateContent"):
            self._body()
            self._send(HTTPStatus.OK, _google_stream_body(), "text/event-stream")
        elif path == "/v1/messages":
            payload = self._json_body()
            if payload.get("stream") is True:
                self._send(HTTPStatus.OK, _anthropic_stream_body(), "text/event-stream")
            else:
                model = (
                    payload.get("model")
                    if isinstance(payload.get("model"), str)
                    else "claude-sonnet-4-20250514"
                )
                if _anthropic_has_tool_result(payload):
                    self._send_json(_anthropic_final_payload(model, payload))
                elif payload.get("tools"):
                    self._send_json(_anthropic_tool_use_payload(model, payload))
                else:
                    self._send_json(_anthropic_message_payload(model))
        elif path == "/api/chat":
            payload = self._json_body()
            model = payload.get("model") if isinstance(payload.get("model"), str) else "gemma4:latest"
            if _ollama_has_tool_result(payload):
                self._send_json(_ollama_chat_final_payload(model, payload))
            elif payload.get("tools"):
                self._send_json(_ollama_chat_tool_payload(model, payload))
            else:
                self._send_json(_ollama_chat_payload(model))
        elif path == "/api/show":
            payload = self._json_body()
            model = payload.get("model") if isinstance(payload.get("model"), str) else "gemma4:latest"
            self._send_json(
                {
                    "license": "capsem-mock",
                    "modelfile": f"FROM {model}",
                    "parameters": "num_ctx 8192",
                    "template": "{{ .Prompt }}",
                    "details": {
                        "format": "gguf",
                        "family": "gemma",
                        "families": ["gemma"],
                        "parameter_size": "7B",
                        "quantization_level": "Q4_0",
                    },
                    "model_info": {"general.architecture": "gemma"},
                }
            )
        elif path == "/oauth/token":
            self._body()
            self._send_json(
                {
                    "kind": "synthetic_oauth_token_fixture",
                    "token_type": "Bearer",
                    "access_token": "capsem_test_oauth_access_0123456789abcdef",
                    "refresh_token": "capsem_test_oauth_refresh_0123456789abcdef",
                    "id_token": "capsem_test_oauth_id_0123456789abcdef",
                    "expires_in": 3600,
                    "scope": "openid profile email offline_access",
                }
            )
        elif path == "/mcp":
            self._mcp(self._json_body())
        elif path == "/echo":
            body = self._body()
            lower_headers = {key.lower(): value for key, value in self.headers.items()}
            authorization = lower_headers.get("authorization", "")
            self._send_json(
                {
                    "method": "POST",
                    "path": "/echo",
                    "body_size": len(body),
                    "content_type": lower_headers.get("content-type"),
                    "user_agent": lower_headers.get("user-agent"),
                    "header_count": len(self.headers),
                    "has_authorization": "authorization" in lower_headers,
                    "authorization_is_broker_ref": "credential:blake3:" in authorization,
                    "query_has_broker_ref": "credential:blake3:" in parsed.query,
                    "query_has_access_token": "access_token=" in parsed.query,
                    "has_cookie": "cookie" in lower_headers,
                    "has_x_api_key": "x-api-key" in lower_headers,
                }
            )
        else:
            self._send_json({"error": "not found"}, HTTPStatus.NOT_FOUND)

    def _bytes(self, size: str, *, gzip_body: bool) -> None:
        try:
            data = _deterministic_bytes(size)
        except ValueError as exc:
            self._send_json({"error": str(exc), "allowed": ["10kb", "1mb", "10mb"]}, 400)
            return
        if gzip_body:
            data = gzip.compress(data)
            self.send_response(HTTPStatus.OK)
            self.send_header("content-type", "application/octet-stream")
            self.send_header("content-encoding", "gzip")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        else:
            self._send(HTTPStatus.OK, data, "application/octet-stream")

    def _mcp(self, payload: dict) -> None:
        request_id = payload.get("id")
        method = payload.get("method")
        if method == "initialize":
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {"listChanged": False}, "resources": {}},
                    "serverInfo": {"name": "capsem-mock-server", "version": "1.0.0"},
                },
            }
        elif method == "tools/list":
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "tools": [
                        {
                            "name": "fixture_lookup",
                            "description": "Return deterministic debug content.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"query": {"type": "string"}},
                            },
                        },
                        {
                            "name": "fetch_http",
                            "description": "Fetch a local mock server URL.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"url": {"type": "string"}},
                            },
                        },
                        {
                            "name": "slow_sleep",
                            "description": "Sleep before returning deterministic text.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {},
                            },
                        },
                    ]
                },
            }
        elif method == "tools/call":
            name = payload.get("params", {}).get("name", "unknown")
            if name == "slow_sleep":
                time.sleep(3)
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "content": [
                        {"type": "text", "text": f"capsem-mock-server:mcp:{name}"}
                    ],
                    "isError": False,
                },
            }
        elif method == "resources/list":
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "resources": [
                        {
                            "uri": "doc://slow",
                            "name": "slow-doc",
                            "description": "Slow deterministic resource.",
                            "mimeType": "text/plain",
                        }
                    ]
                },
            }
        elif method == "resources/read":
            if payload.get("params", {}).get("uri") == "doc://slow":
                time.sleep(3)
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "contents": [
                        {
                            "uri": payload.get("params", {}).get("uri", "doc://unknown"),
                            "mimeType": "text/plain",
                            "text": "capsem-mock-server:mcp:resource",
                        }
                    ]
                },
            }
        else:
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {"code": -32601, "message": "method not found"},
            }
        self._send_json(response)

    def _websocket(self, path: str) -> None:
        key = self.headers.get("Sec-WebSocket-Key")
        if not key:
            self.send_error(HTTPStatus.BAD_REQUEST)
            return
        accept = base64.b64encode(
            hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()
        ).decode()
        self.send_response(HTTPStatus.SWITCHING_PROTOCOLS)
        self.send_header("upgrade", "websocket")
        self.send_header("connection", "Upgrade")
        self.send_header("sec-websocket-accept", accept)
        self.end_headers()
        if path == "/ws/close":
            self._ws_send_close()
            return
        if path == "/ws/ping":
            self._ws_send_frame(0x9, b"capsem-ping")
        if path != "/ws/echo":
            return
        while True:
            frame = self._ws_read_frame()
            if frame is None:
                return
            opcode, payload = frame
            if opcode == 0x8:
                self._ws_send_close()
                return
            if opcode in {0x1, 0x2}:
                self._ws_send_frame(opcode, payload)
            elif opcode == 0x9:
                self._ws_send_frame(0xA, payload)

    def _ws_read_frame(self) -> tuple[int, bytes] | None:
        head = self.connection.recv(2)
        if len(head) < 2:
            return None
        first, second = head
        opcode = first & 0x0F
        masked = second & 0x80
        length = second & 0x7F
        if length == 126:
            length = struct.unpack("!H", self.connection.recv(2))[0]
        elif length == 127:
            length = struct.unpack("!Q", self.connection.recv(8))[0]
        mask = self.connection.recv(4) if masked else b"\0\0\0\0"
        payload = bytearray()
        while len(payload) < length:
            chunk = self.connection.recv(length - len(payload))
            if not chunk:
                return None
            payload.extend(chunk)
        if masked:
            payload = bytearray(byte ^ mask[idx % 4] for idx, byte in enumerate(payload))
        return opcode, bytes(payload)

    def _ws_send_frame(self, opcode: int, payload: bytes) -> None:
        header = bytearray([0x80 | opcode])
        length = len(payload)
        if length < 126:
            header.append(length)
        elif length <= 0xFFFF:
            header.extend([126])
            header.extend(struct.pack("!H", length))
        else:
            header.extend([127])
            header.extend(struct.pack("!Q", length))
        self.connection.sendall(bytes(header) + payload)

    def _ws_send_close(self) -> None:
        self._ws_send_frame(0x8, struct.pack("!H", 1000) + b"capsem-fixture-close")


def _decode_dns_name(packet: bytes, offset: int = 12) -> tuple[str, int]:
    labels: list[str] = []
    while True:
        if offset >= len(packet):
            raise ValueError("truncated dns name")
        length = packet[offset]
        offset += 1
        if length == 0:
            break
        if length & 0xC0:
            raise ValueError("compressed dns query names are unsupported in fixtures")
        if offset + length > len(packet):
            raise ValueError("truncated dns label")
        labels.append(packet[offset:offset + length].decode("ascii").lower())
        offset += length
    return ".".join(labels), offset


def _dns_response(packet: bytes) -> bytes:
    if len(packet) < 12:
        return b""
    query_id, _flags, qdcount, _ancount, _nscount, _arcount = struct.unpack("!HHHHHH", packet[:12])
    if qdcount != 1:
        return struct.pack("!HHHHHH", query_id, 0x8183, qdcount, 0, 0, 0) + packet[12:]
    try:
        qname, offset = _decode_dns_name(packet)
    except ValueError:
        return struct.pack("!HHHHHH", query_id, 0x8183, 0, 0, 0, 0)
    if offset + 4 > len(packet):
        return struct.pack("!HHHHHH", query_id, 0x8183, 0, 0, 0, 0)
    qtype, qclass = struct.unpack("!HH", packet[offset:offset + 4])
    question = packet[12:offset + 4]
    address = DNS_FIXTURES.get(qname)
    if qtype != 1 or qclass != 1 or address is None:
        return struct.pack("!HHHHHH", query_id, 0x8183, 1, 0, 0, 0) + question
    rdata = bytes(int(part) for part in address.split("."))
    answer = (
        struct.pack("!HHHIH", 0xC00C, 1, 1, 60, len(rdata))
        + rdata
    )
    return struct.pack("!HHHHHH", query_id, 0x8180, 1, 1, 0, 0) + question + answer


class DnsUdpHandler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        data, socket = self.request
        response = _dns_response(data)
        if response:
            socket.sendto(response, self.client_address)


class DnsTcpHandler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        length_bytes = self.request.recv(2)
        if len(length_bytes) != 2:
            return
        length = struct.unpack("!H", length_bytes)[0]
        packet = b""
        while len(packet) < length:
            chunk = self.request.recv(length - len(packet))
            if not chunk:
                return
            packet += chunk
        response = _dns_response(packet)
        if response:
            self.request.sendall(struct.pack("!H", len(response)) + response)


class ThreadingUdpServer(socketserver.ThreadingMixIn, socketserver.UDPServer):
    daemon_threads = True
    allow_reuse_address = True


class ThreadingTcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    daemon_threads = True
    allow_reuse_address = True


def _ready_payload(
    http_addr: tuple[str, int],
    https_addr: tuple[str, int],
    dns_udp_addr: tuple[str, int],
    dns_tcp_addr: tuple[str, int],
) -> dict:
    host, port = http_addr
    https_host, https_port = https_addr
    dns_udp_host, dns_udp_port = dns_udp_addr
    dns_tcp_host, dns_tcp_port = dns_tcp_addr
    return {
        "service": "capsem-mock-server",
        "http_addr": f"{host}:{port}",
        "base_url": f"http://{host}:{port}",
        "https_addr": f"{https_host}:{https_port}",
        "https_base_url": f"https://{https_host}:{https_port}",
        "dns_udp_addr": f"{dns_udp_host}:{dns_udp_port}",
        "dns_tcp_addr": f"{dns_tcp_host}:{dns_tcp_port}",
        "dns_fixtures": sorted(DNS_FIXTURES),
        "endpoints": ENDPOINTS,
        "request_log": str(REQUEST_LOG_PATH) if REQUEST_LOG_PATH is not None else None,
    }


def _tls_context(tmpdir: Path) -> ssl.SSLContext:
    key_path = tmpdir / "mock-server.key"
    cert_path = tmpdir / "mock-server.crt"
    subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            str(key_path),
            "-out",
            str(cert_path),
            "-sha256",
            "-days",
            "1",
            "-subj",
            "/CN=127.0.0.1",
            "-addext",
            "subjectAltName=IP:127.0.0.1,DNS:localhost",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certfile=cert_path, keyfile=key_path)
    return context


def main() -> int:
    global REQUEST_LOG_PATH
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--addr", default="127.0.0.1:0")
    parser.add_argument("--request-log", default=None)
    args = parser.parse_args()
    REQUEST_LOG_PATH = Path(args.request_log) if args.request_log else None
    host, port_text = args.addr.rsplit(":", 1)
    server = ThreadingHTTPServer((host, int(port_text)), MockHandler)
    tls_tmpdir = tempfile.TemporaryDirectory(prefix="capsem-mock-server-tls-")
    tls_context = _tls_context(Path(tls_tmpdir.name))
    https_server = ThreadingHTTPServer((host, 0), MockHandler)
    https_server.socket = tls_context.wrap_socket(https_server.socket, server_side=True)
    dns_udp = ThreadingUdpServer((host, 0), DnsUdpHandler)
    dns_tcp = ThreadingTcpServer((host, 0), DnsTcpHandler)
    print(
        json.dumps(
            _ready_payload(
                server.server_address,
                https_server.server_address,
                dns_udp.server_address,
                dns_tcp.server_address,
            )
        ),
        flush=True,
    )
    threads = [
        threading.Thread(target=server.serve_forever, daemon=True),
        threading.Thread(target=https_server.serve_forever, daemon=True),
        threading.Thread(target=dns_udp.serve_forever, daemon=True),
        threading.Thread(target=dns_tcp.serve_forever, daemon=True),
    ]
    for thread in threads:
        thread.start()
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        pass
    finally:
        for fixture_server in (server, https_server, dns_udp, dns_tcp):
            fixture_server.shutdown()
            fixture_server.server_close()
        tls_tmpdir.cleanup()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except OSError as exc:
        print(f"capsem-mock-server failed: {exc}", file=sys.stderr)
        raise
