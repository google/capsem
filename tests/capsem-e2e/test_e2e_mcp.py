"""E2E MCP tests: real capsem-mcp binary over stdio JSON-RPC.

Spawns the actual capsem-mcp binary, sends JSON-RPC over stdin,
reads responses from stdout. Tests the full MCP stack: binary startup,
service connection, VM lifecycle, tool execution.
"""

import json
import os
import subprocess
import sys
import time
import uuid

import pytest

from pathlib import Path
from .conftest import RealService

PROJECT_ROOT = Path(__file__).parent.parent.parent
MCP_BINARY = PROJECT_ROOT / "target/debug/capsem-mcp"

pytestmark = pytest.mark.e2e


class McpClient:
    """Minimal JSON-RPC client over stdio to the real capsem-mcp binary."""

    def __init__(self, proc):
        self.proc = proc
        self._id = 1

    def request(self, method, params=None):
        req = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {},
            "id": self._id,
        }
        self._id += 1
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()

        line = self.proc.stdout.readline()
        if not line:
            raise EOFError("MCP server closed stdout")
        return json.loads(line)

    def notify(self, method, params=None):
        req = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {},
        }
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()

    def call_tool(self, name, args=None):
        resp = self.request("tools/call", {"name": name, "arguments": args or {}})
        assert "error" not in resp, f"JSON-RPC error: {resp.get('error')}"
        result = resp["result"]
        assert not result.get("isError"), f"Tool error: {result.get('content')}"
        return result

    def tool_text(self, name, args=None):
        result = self.call_tool(name, args)
        return result["content"][0]["text"]

    def tool_json(self, name, args=None):
        return json.loads(self.tool_text(name, args))


def _start_mcp(uds_path):
    env = os.environ.copy()
    env["CAPSEM_UDS_PATH"] = str(uds_path)
    env["CAPSEM_RUN_DIR"] = str(Path(uds_path).parent)

    proc = subprocess.Popen(
        [str(MCP_BINARY)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        text=True,
        bufsize=1,
        env=env,
    )

    client = McpClient(proc)
    client.request("initialize", {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "e2e-test", "version": "1.0"},
    })
    client.notify("notifications/initialized")
    return client, proc


class TestMcpLifecycle:

    def test_create_list_delete(self, service):
        """Full MCP lifecycle: create VM, list it, delete it."""
        client, proc = _start_mcp(service.uds_path)
        try:
            name = f"mcp-{uuid.uuid4().hex[:8]}"

            # Create
            result = client.tool_json("capsem_create", {"name": name})
            assert result.get("id") == name or name in str(result)

            # List
            result = client.tool_json("capsem_list")
            ids = [s["id"] for s in result.get("sandboxes", [])]
            assert name in ids

            # Delete
            client.call_tool("capsem_delete", {"id": name})

            # Verify gone
            result = client.tool_json("capsem_list")
            ids = [s["id"] for s in result.get("sandboxes", [])]
            assert name not in ids
        finally:
            proc.terminate()
            proc.wait(timeout=5)

    def test_exec_via_mcp(self, service):
        """MCP exec returns correct output from VM."""
        client, proc = _start_mcp(service.uds_path)
        try:
            name = f"mcp-exec-{uuid.uuid4().hex[:8]}"
            client.call_tool("capsem_create", {"name": name})

            # Wait for exec ready
            ready = False
            for _ in range(30):
                try:
                    text = client.tool_text("capsem_exec", {
                        "id": name, "command": "echo mcp-ready",
                    })
                    if "mcp-ready" in text:
                        ready = True
                        break
                except (AssertionError, KeyError):
                    pass
                time.sleep(1)
            assert ready, f"VM {name} never exec-ready via MCP"

            # Actual test
            text = client.tool_text("capsem_exec", {
                "id": name, "command": "echo mcp-works",
            })
            assert "mcp-works" in text

            client.call_tool("capsem_delete", {"id": name})
        finally:
            proc.terminate()
            proc.wait(timeout=5)

    def test_file_io_via_mcp(self, service):
        """MCP write_file + read_file roundtrip."""
        client, proc = _start_mcp(service.uds_path)
        try:
            name = f"mcp-fio-{uuid.uuid4().hex[:8]}"
            client.call_tool("capsem_create", {"name": name})

            # Wait for ready
            for _ in range(30):
                try:
                    text = client.tool_text("capsem_exec", {
                        "id": name, "command": "echo ready",
                    })
                    if "ready" in text:
                        break
                except (AssertionError, KeyError):
                    pass
                time.sleep(1)

            # Write
            client.call_tool("capsem_write_file", {
                "id": name, "path": "/root/mcp-test.txt", "content": "mcp-payload",
            })

            # Read
            text = client.tool_text("capsem_read_file", {
                "id": name, "path": "/root/mcp-test.txt",
            })
            assert "mcp-payload" in text

            client.call_tool("capsem_delete", {"id": name})
        finally:
            proc.terminate()
            proc.wait(timeout=5)

    def test_tools_list(self, service):
        """MCP server reports available tools."""
        client, proc = _start_mcp(service.uds_path)
        try:
            resp = client.request("tools/list")
            tools = resp["result"]["tools"]
            tool_names = [t["name"] for t in tools]
            assert "capsem_create" in tool_names
            assert "capsem_exec" in tool_names
            assert "capsem_delete" in tool_names
            assert "capsem_list" in tool_names
        finally:
            proc.terminate()
            proc.wait(timeout=5)
