"""Bounded synthetic read-only corpus using existing log/cache contracts.

MCP cache metadata follows capsem-service/src/tests/mcp_routes.rs. It grants no
approval and invokes no tool. Logs are plain text on their existing log-file
boundary; a separate retained service generation never rewrites the active sink.
"""
from __future__ import annotations

import json
from pathlib import Path

import tomllib
from helpers.stopped_workspace import VM_ID

ROOT = Path(__file__).resolve().parents[2]
MARKER = "CAPSEM-ROUTE-CORPUS"


def corpus_config() -> dict:
    return tomllib.loads((ROOT / "config/gate.toml").read_text())["benchmark"]["routes"]["corpus"]


def seed_metadata(service, cfg: dict) -> None:
    cache = service.home_dir / "mcp_tool_cache.json"
    assert not cache.exists(), "route corpus needs a fresh isolated MCP cache"
    # This is the existing serialized cache fixture, not network traffic or an
    # injection record. JSON is required by its owning Rust load_tool_cache.
    cache.write_text(json.dumps([{
        "namespaced_name": f"local__{name}", "original_name": name,
        "description": "Local builtin cached tool", "server_name": "local",
        "annotations": None, "pin_hash": "route-cache-metadata-only",
        "first_seen": "2026-01-01T00:00:00Z", "last_seen": "2026-01-01T00:00:00Z",
        "approved": False,
    } for name in cfg["mcp_tools"]]))
    cache.chmod(0o600)


def seed_logs(service, cfg: dict) -> None:
    size, width = cfg["log_bytes"], cfg["log_line_bytes"]
    assert size % width == 0 and width >= 64
    lines = []
    for index in range(size // width):
        text = f"{MARKER} {index:08d} "
        lines.append((text.ljust(width - 1, "x") + "\n").encode())
    content = b"".join(lines)
    session = service.tmp_dir / "persistent" / VM_ID
    for path in (session / "serial.log", session / "process.log", service.tmp_dir / "mcp.log", service.tmp_dir / "service.benchmark.log"):
        assert not path.exists(), f"refusing to replace an existing log: {path}"
        path.write_bytes(content)
        path.chmod(0o600)
