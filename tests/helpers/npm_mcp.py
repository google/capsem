"""Packed npm MCP process helpers for gateway acceptance tests."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from .mcp import kill_mcp_proc

PROJECT_ROOT = Path(__file__).resolve().parents[2]
PACKAGE_ROOT = PROJECT_ROOT / "mcp" / "typescript"


class McpSession:
    """Small JSON-RPC client for the public stdio MCP protocol."""

    def __init__(self, proc: subprocess.Popen[str]):
        self.proc = proc
        self._next_id = 1

    def request(self, method: str, params: dict | None = None) -> dict:
        request = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {},
            "id": self._next_id,
        }
        self._next_id += 1
        assert self.proc.stdin is not None
        assert self.proc.stdout is not None
        self.proc.stdin.write(json.dumps(request, separators=(",", ":")) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        assert line, "capsem-mcp closed stdout"
        return json.loads(line)

    def notify(self, method: str, params: dict | None = None) -> None:
        request = {"jsonrpc": "2.0", "method": method, "params": params or {}}
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(request, separators=(",", ":")) + "\n")
        self.proc.stdin.flush()

    def call_tool(self, name: str, args: dict | None = None) -> dict:
        response = self.request("tools/call", {"name": name, "arguments": args or {}})
        assert "error" not in response, response
        result = response["result"]
        assert result.get("isError") is not True, result
        return result


def structured(result: dict) -> object:
    """Return a typed MCP result while retaining text compatibility."""
    if "structuredContent" in result:
        return result["structuredContent"]
    content = result.get("content", [])
    assert content and content[0].get("type") == "text", result
    return json.loads(content[0]["text"])


def _pack() -> tuple[Path, Path]:
    fixture = Path(tempfile.mkdtemp(prefix="capsem-mcp-pack-"))
    packed = subprocess.run(
        ["pnpm", "pack", "--pack-destination", str(fixture)],
        cwd=PACKAGE_ROOT,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    assert packed.returncode == 0, packed.stdout + packed.stderr
    archives = list(fixture.glob("*.tgz"))
    assert len(archives) == 1, archives
    with tarfile.open(archives[0], "r:gz") as archive:
        archive.extractall(fixture, filter="data")
    extracted = fixture / "package"
    (extracted / "node_modules").symlink_to(
        PACKAGE_ROOT / "node_modules", target_is_directory=True
    )
    return fixture, extracted / "dist" / "cli.js"


@contextmanager
def packed_npm_mcp(*, gateway_url: str, access_id: str, capability: str) -> Iterator[McpSession]:
    """Launch only with explicit operator-issued delegated authority."""
    fixture, cli = _pack()
    proc = subprocess.Popen(
        [
            "node",
            str(cli),
            "--gateway-url",
            gateway_url,
            "--access-id",
            access_id,
            "--timeout-ms",
            "120000",
        ],
        env={
            **{key: value for key, value in os.environ.items()
               if key not in {"CAPSEM_GATEWAY_TOKEN", "CAPSEM_GATEWAY_TOKEN_FILE"}},
            "CAPSEM_AGENT_CAPABILITY": capability,
        },
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        text=True,
        bufsize=1,
    )
    session = McpSession(proc)
    try:
        initialized = session.request(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "capsem-packed-acceptance", "version": "1.0"},
            },
        )
        assert initialized["result"]["serverInfo"]["name"] == "capsem-mcp"
        session.notify("notifications/initialized")
        yield session
    finally:
        kill_mcp_proc(proc)
        shutil.rmtree(fixture, ignore_errors=True)
