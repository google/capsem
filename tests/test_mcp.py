"""Tests for capsem.builder.mcp_server -- JSON-RPC 2.0 MCP stdio server.

TDD: tests written first (RED), then mcp_server.py makes them pass (GREEN).
Uses in-process stream injection (io.StringIO) for testing -- no subprocess.
"""

from __future__ import annotations

import io
import json
import textwrap
from pathlib import Path

import pytest

from capsem.builder.mcp_server import BuilderMcpServer

PROJECT_ROOT = Path(__file__).parent.parent

# ---------------------------------------------------------------------------
# Inline TOML fixtures (for tools/call tests that need a guest dir)
# ---------------------------------------------------------------------------

MINIMAL_BUILD_TOML = """\
[build]
compression = "zstd"
compression_level = 15

[build.architectures.arm64]
base_image = "debian:bookworm-slim"
docker_platform = "linux/arm64"
rust_target = "aarch64-unknown-linux-musl"
kernel_branch = "6.6"
kernel_image = "arch/arm64/boot/Image"
defconfig = "kernel/defconfig.arm64"
node_major = 24
"""

TRIVY_JSON = json.dumps({
    "Results": [{
        "Target": "test",
        "Vulnerabilities": [{
            "VulnerabilityID": "CVE-2024-1234",
            "Severity": "HIGH",
            "PkgName": "openssl",
            "InstalledVersion": "3.0.13",
            "FixedVersion": "3.0.14",
        }],
    }],
})


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _roundtrip(messages: list[dict]) -> list[dict]:
    """Send NDJSON messages to BuilderMcpServer, collect responses."""
    input_text = "\n".join(json.dumps(m) for m in messages) + "\n"
    input_stream = io.StringIO(input_text)
    output_stream = io.StringIO()
    server = BuilderMcpServer(input_stream=input_stream, output_stream=output_stream)
    server.run()
    responses = []
    for line in output_stream.getvalue().strip().splitlines():
        if line.strip():
            responses.append(json.loads(line))
    return responses


def _init_messages() -> list[dict]:
    """Standard initialize + notifications/initialized sequence."""
    return [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"},
        }},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
    ]


def _write_minimal_guest(tmp_path: Path) -> Path:
    guest = tmp_path / "guest"
    config = guest / "config"
    config.mkdir(parents=True)
    (config / "build.toml").write_text(MINIMAL_BUILD_TOML)
    kernel_dir = config / "kernel"
    kernel_dir.mkdir()
    (kernel_dir / "defconfig.arm64").write_text("# minimal\n")
    return guest


# ---------------------------------------------------------------------------
# Initialize
# ---------------------------------------------------------------------------


class TestMcpInitialize:

    def test_returns_server_info(self):
        msgs = [_init_messages()[0]]
        responses = _roundtrip(msgs)
        assert len(responses) == 1
        result = responses[0]["result"]
        assert result["serverInfo"]["name"] == "capsem-builder"

    def test_protocol_version(self):
        responses = _roundtrip([_init_messages()[0]])
        assert responses[0]["result"]["protocolVersion"] == "2024-11-05"

    def test_capabilities_include_tools(self):
        responses = _roundtrip([_init_messages()[0]])
        assert "tools" in responses[0]["result"]["capabilities"]

    def test_response_id_matches(self):
        msg = {"jsonrpc": "2.0", "id": 42, "method": "initialize", "params": {
            "protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"},
        }}
        responses = _roundtrip([msg])
        assert responses[0]["id"] == 42


# ---------------------------------------------------------------------------
# tools/list
# ---------------------------------------------------------------------------


class TestMcpToolsList:

    def test_returns_tools(self):
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
        ]
        responses = _roundtrip(msgs)
        # initialize response + tools/list response (notification has no response)
        tools_resp = [r for r in responses if r.get("id") == 2][0]
        tools = tools_resp["result"]["tools"]
        assert len(tools) >= 4

    def test_tool_names(self):
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
        ]
        responses = _roundtrip(msgs)
        tools_resp = [r for r in responses if r.get("id") == 2][0]
        names = {t["name"] for t in tools_resp["result"]["tools"]}
        assert "validate" in names
        assert "build_dry_run" in names
        assert "inspect" in names
        assert "audit_parse" in names

    def test_tools_have_input_schema(self):
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
        ]
        responses = _roundtrip(msgs)
        tools_resp = [r for r in responses if r.get("id") == 2][0]
        for tool in tools_resp["result"]["tools"]:
            assert "inputSchema" in tool

    def test_before_initialize_errors(self):
        msgs = [{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}]
        responses = _roundtrip(msgs)
        assert "error" in responses[0]


# ---------------------------------------------------------------------------
# tools/call
# ---------------------------------------------------------------------------


class TestMcpToolsCall:

    def test_validate_tool(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "validate", "arguments": {"guest_dir": str(guest)},
            }},
        ]
        responses = _roundtrip(msgs)
        call_resp = [r for r in responses if r.get("id") == 3][0]
        assert "result" in call_resp
        assert call_resp["result"]["isError"] is False

    def test_inspect_tool(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "inspect", "arguments": {"guest_dir": str(guest)},
            }},
        ]
        responses = _roundtrip(msgs)
        call_resp = [r for r in responses if r.get("id") == 3][0]
        result_text = call_resp["result"]["content"][0]["text"]
        # Should be valid JSON (inspect returns config dump)
        data = json.loads(result_text)
        assert "build" in data

    def test_audit_parse_tool(self):
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "audit_parse",
                "arguments": {"output": TRIVY_JSON, "scanner": "trivy"},
            }},
        ]
        responses = _roundtrip(msgs)
        call_resp = [r for r in responses if r.get("id") == 3][0]
        result_text = call_resp["result"]["content"][0]["text"]
        data = json.loads(result_text)
        assert len(data) == 1
        assert data[0]["id"] == "CVE-2024-1234"

    def test_build_dry_run_tool(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "build_dry_run",
                "arguments": {"guest_dir": str(guest), "arch": "arm64"},
            }},
        ]
        responses = _roundtrip(msgs)
        call_resp = [r for r in responses if r.get("id") == 3][0]
        assert "FROM" in call_resp["result"]["content"][0]["text"]

    def test_unknown_tool(self):
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "nonexistent", "arguments": {},
            }},
        ]
        responses = _roundtrip(msgs)
        call_resp = [r for r in responses if r.get("id") == 3][0]
        assert call_resp["result"]["isError"] is True

    def test_bad_params(self):
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "validate", "arguments": {"guest_dir": "/nonexistent/path"},
            }},
        ]
        responses = _roundtrip(msgs)
        call_resp = [r for r in responses if r.get("id") == 3][0]
        assert call_resp["result"]["isError"] is True


# ---------------------------------------------------------------------------
# Protocol edge cases
# ---------------------------------------------------------------------------


class TestMcpProtocol:

    def test_invalid_json(self):
        input_stream = io.StringIO("not json\n")
        output_stream = io.StringIO()
        server = BuilderMcpServer(input_stream=input_stream, output_stream=output_stream)
        server.run()
        responses = [json.loads(l) for l in output_stream.getvalue().strip().splitlines() if l.strip()]
        assert responses[0]["error"]["code"] == -32700

    def test_missing_method(self):
        msgs = [{"jsonrpc": "2.0", "id": 1}]
        responses = _roundtrip(msgs)
        assert responses[0]["error"]["code"] == -32600

    def test_unknown_method(self):
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 5, "method": "unknown/method"},
        ]
        responses = _roundtrip(msgs)
        unknown_resp = [r for r in responses if r.get("id") == 5][0]
        assert unknown_resp["error"]["code"] == -32601

    def test_notification_no_response(self):
        # notifications/initialized has no id, should produce no response
        msgs = _init_messages()
        responses = _roundtrip(msgs)
        # Only the initialize response (id=1), not the notification
        assert len(responses) == 1
        assert responses[0]["id"] == 1

    def test_empty_input(self):
        input_stream = io.StringIO("")
        output_stream = io.StringIO()
        server = BuilderMcpServer(input_stream=input_stream, output_stream=output_stream)
        server.run()
        assert output_stream.getvalue() == ""

    def test_multiple_requests(self):
        msgs = _init_messages() + [
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/list"},
        ]
        responses = _roundtrip(msgs)
        ids = {r.get("id") for r in responses}
        assert {1, 2, 3} == ids
