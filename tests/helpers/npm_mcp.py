"""Packed npm MCP process helpers for gateway acceptance tests."""

from __future__ import annotations

import json
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
def packed_npm_mcp(run_dir: Path) -> Iterator[McpSession]:
    """Launch the packed npm MCP against one service-owned gateway."""
    port_path = run_dir / "gateway.port"
    token_path = run_dir / "gateway.token"
    assert port_path.exists() and token_path.exists(), (
        "service gateway credentials missing"
    )
    fixture, cli = _pack()
    proc = subprocess.Popen(
        [
            "node",
            str(cli),
            "--gateway-url",
            f"http://127.0.0.1:{port_path.read_text().strip()}",
            "--token-file",
            str(token_path),
            "--timeout-ms",
            "120000",
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        text=True,
        bufsize=1,
    )
    session = McpSession(proc)
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
    try:
        yield session
    finally:
        kill_mcp_proc(proc)
        shutil.rmtree(fixture, ignore_errors=True)
