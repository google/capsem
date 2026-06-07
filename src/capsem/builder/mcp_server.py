"""JSON-RPC 2.0 MCP stdio server for capsem-builder tools.

Thin wrapper exposing builder functions (validate, inspect, build dry-run,
audit parse) over the MCP protocol. Uses stdlib json for NDJSON on
stdin/stdout -- no external MCP library.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any, TextIO

from capsem.builder.audit import parse_audit_output
from capsem.builder.config import load_guest_config
from capsem.builder.docker import render_dockerfile
from capsem.builder.validate import Severity, validate_guest


# ---------------------------------------------------------------------------
# Tool definitions
# ---------------------------------------------------------------------------

_TOOLS = [
    {
        "name": "validate",
        "description": "Validate a guest image configuration directory.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "guest_dir": {"type": "string", "description": "Path to guest directory"},
            },
            "required": ["guest_dir"],
        },
    },
    {
        "name": "build_dry_run",
        "description": "Render a Dockerfile from config (dry run).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "guest_dir": {"type": "string", "description": "Path to guest directory"},
                "arch": {"type": "string", "description": "Architecture (e.g. arm64)"},
                "template": {"type": "string", "enum": ["rootfs", "kernel"], "default": "rootfs"},
            },
            "required": ["guest_dir"],
        },
    },
    {
        "name": "inspect",
        "description": "Show guest config as JSON.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "guest_dir": {"type": "string", "description": "Path to guest directory"},
            },
            "required": ["guest_dir"],
        },
    },
    {
        "name": "audit_parse",
        "description": "Parse vulnerability scanner JSON output.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "output": {"type": "string", "description": "Scanner JSON output"},
                "scanner": {"type": "string", "enum": ["trivy", "grype"]},
            },
            "required": ["output", "scanner"],
        },
    },
]


# ---------------------------------------------------------------------------
# Tool dispatch
# ---------------------------------------------------------------------------


def _call_validate(args: dict) -> str:
    guest_dir = Path(args["guest_dir"])
    if not guest_dir.is_dir():
        raise ValueError(f"Directory not found: {guest_dir}")
    diags = validate_guest(guest_dir)
    errors = [d for d in diags if d.severity == Severity.ERROR]
    warnings = [d for d in diags if d.severity == Severity.WARNING]
    lines = [str(d) for d in diags]
    if errors:
        lines.append(f"\n{len(errors)} error(s), {len(warnings)} warning(s)")
    elif warnings:
        lines.append(f"\n{len(warnings)} warning(s), 0 errors -- passed")
    else:
        lines.append("passed: config is clean")
    return "\n".join(lines)


def _call_build_dry_run(args: dict) -> str:
    guest_dir = Path(args["guest_dir"])
    if not guest_dir.is_dir():
        raise ValueError(f"Directory not found: {guest_dir}")
    config = load_guest_config(guest_dir)
    template = args.get("template", "rootfs")
    template_name = f"Dockerfile.{template}.j2"
    arch = args.get("arch")
    if arch is None:
        arch = next(iter(config.build.architectures))
    if arch not in config.build.architectures:
        avail = ", ".join(config.build.architectures.keys())
        raise ValueError(f"Architecture '{arch}' not found (available: {avail})")
    return render_dockerfile(template_name, config, arch)


def _call_inspect(args: dict) -> str:
    guest_dir = Path(args["guest_dir"])
    if not guest_dir.is_dir():
        raise ValueError(f"Directory not found: {guest_dir}")
    config = load_guest_config(guest_dir)
    return json.dumps(config.model_dump(mode="json"), indent=2)


def _call_audit_parse(args: dict) -> str:
    output = args["output"]
    scanner = args["scanner"]
    vulns = parse_audit_output(output, scanner)
    return json.dumps([v.model_dump() for v in vulns], indent=2)


_TOOL_HANDLERS = {
    "validate": _call_validate,
    "build_dry_run": _call_build_dry_run,
    "inspect": _call_inspect,
    "audit_parse": _call_audit_parse,
}


# ---------------------------------------------------------------------------
# Server
# ---------------------------------------------------------------------------


def _get_version() -> str:
    try:
        from importlib.metadata import version
        return version("capsem")
    except Exception:
        return "0.0.0"


class BuilderMcpServer:
    """MCP stdio server exposing capsem-builder tools."""

    def __init__(
        self,
        input_stream: TextIO | None = None,
        output_stream: TextIO | None = None,
    ):
        self._input = input_stream or sys.stdin
        self._output = output_stream or sys.stdout
        self._initialized = False

    def _write(self, msg: dict) -> None:
        self._output.write(json.dumps(msg) + "\n")
        self._output.flush()

    def _error_response(self, id: Any, code: int, message: str) -> dict:
        return {"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}

    def _result_response(self, id: Any, result: Any) -> dict:
        return {"jsonrpc": "2.0", "id": id, "result": result}

    def _handle_initialize(self, id: Any, params: dict) -> dict:
        self._initialized = True
        return self._result_response(id, {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {"listChanged": False}},
            "serverInfo": {"name": "capsem-builder", "version": _get_version()},
        })

    def _handle_tools_list(self, id: Any) -> dict:
        if not self._initialized:
            return self._error_response(id, -32600, "Server not initialized")
        return self._result_response(id, {"tools": _TOOLS})

    def _handle_tools_call(self, id: Any, params: dict) -> dict:
        if not self._initialized:
            return self._error_response(id, -32600, "Server not initialized")
        name = params.get("name", "")
        args = params.get("arguments", {})
        handler = _TOOL_HANDLERS.get(name)
        if handler is None:
            return self._result_response(id, {
                "content": [{"type": "text", "text": f"Unknown tool: {name}"}],
                "isError": True,
            })
        try:
            result_text = handler(args)
            return self._result_response(id, {
                "content": [{"type": "text", "text": result_text}],
                "isError": False,
            })
        except Exception as e:
            return self._result_response(id, {
                "content": [{"type": "text", "text": str(e)}],
                "isError": True,
            })

    def _handle_message(self, msg: dict) -> dict | None:
        if "method" not in msg:
            id = msg.get("id")
            if id is not None:
                return self._error_response(id, -32600, "Invalid Request: missing method")
            return None

        method = msg["method"]
        id = msg.get("id")
        params = msg.get("params", {})

        if method == "initialize":
            return self._handle_initialize(id, params) if id is not None else None
        if method == "notifications/initialized":
            return None
        if method == "tools/list":
            return self._handle_tools_list(id) if id is not None else None
        if method == "tools/call":
            return self._handle_tools_call(id, params) if id is not None else None

        if id is not None:
            return self._error_response(id, -32601, f"Method not found: {method}")
        return None

    def run(self) -> None:
        """Main loop: read NDJSON messages, dispatch, write responses."""
        for line in self._input:
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                self._write(self._error_response(None, -32700, "Parse error"))
                continue
            response = self._handle_message(msg)
            if response is not None:
                self._write(response)


def run_mcp_server() -> None:
    """Entry point for the MCP stdio server."""
    server = BuilderMcpServer()
    server.run()
