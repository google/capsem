"""Ironbank black-box profile MCP ledger tests."""

from __future__ import annotations

from contextlib import contextmanager
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.mcp import content_text, kill_mcp_proc
from helpers.mock_server import MOCK_SERVER_BINARY, start_mock_server, stop_process
from helpers.service import ServiceInstance, wait_exec_ready, vm_name


PROJECT_ROOT = Path(__file__).resolve().parents[2]
MCP_BINARY = PROJECT_ROOT / "target" / "debug" / "capsem-mcp"
ASSETS_DIR = PROJECT_ROOT / "assets"
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"

pytestmark = pytest.mark.integration

EXPECTED_MCP_SERVER_FIELDS = {
    "name",
    "url",
    "has_auth_credential",
    "custom_header_count",
    "source",
    "enabled",
    "running",
    "tool_count",
    "is_stdio",
}

EXPECTED_MCP_TOOL_FIELDS = {
    "namespaced_name",
    "original_name",
    "description",
    "server_name",
    "annotations",
    "pin_hash",
    "approved",
    "pin_changed",
    "permission_action",
    "permission_source",
}


class McpSession:
    """Tiny JSON-RPC stdio client for the public capsem-mcp server."""

    def __init__(self, proc: subprocess.Popen[str]):
        self.proc = proc
        self._next_id = 1

    def request(self, method: str, params: dict | None = None) -> dict:
        req = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {},
            "id": self._next_id,
        }
        self._next_id += 1
        assert self.proc.stdin is not None
        assert self.proc.stdout is not None
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        assert line, "capsem-mcp closed stdout"
        return json.loads(line)

    def notify(self, method: str, params: dict | None = None) -> None:
        req = {"jsonrpc": "2.0", "method": method, "params": params or {}}
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()

    def call_tool(self, name: str, args: dict | None = None) -> dict:
        resp = self.request("tools/call", {"name": name, "arguments": args or {}})
        assert "error" not in resp, resp
        result = resp["result"]
        assert result.get("isError") is not True, result
        return result


@contextmanager
def _mcp_session(uds_path: Path):
    env = os.environ.copy()
    env["CAPSEM_UDS_PATH"] = str(uds_path)
    env["CAPSEM_RUN_DIR"] = str(uds_path.parent)
    proc = subprocess.Popen(
        [str(MCP_BINARY)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        text=True,
        bufsize=1,
        env=env,
    )
    session = McpSession(proc)
    session.request(
        "initialize",
        {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "ironbank-mcp", "version": "1.0"},
        },
    )
    session.notify("notifications/initialized")
    try:
        yield session
    finally:
        kill_mcp_proc(proc)


def _json_tool_result(result: dict) -> object:
    return json.loads(content_text(result))


@contextmanager
def _connect_session_db(session_root: Path, session_id: str):
    db_path = session_root / session_id / "session.db"
    assert db_path.exists(), f"session DB missing at {db_path}"
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    try:
        yield conn
    finally:
        conn.close()


def _eventually(query, predicate, timeout: float = 5.0):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = query()
        if predicate(last):
            return last
        time.sleep(0.1)
    assert predicate(last), f"condition not met before timeout; last={last!r}"
    return last


def _rows(conn: sqlite3.Connection, sql: str, params: tuple = ()) -> list[sqlite3.Row]:
    return conn.execute(sql, params).fetchall()


def _assert_event_id(value: object) -> None:
    assert isinstance(value, str)
    assert re.fullmatch(r"[0-9a-f]{12}", value), value


def test_profile_mcp_call_pays_full_ledger_blackbox():
    assert MCP_BINARY.exists(), f"{MCP_BINARY} missing; build capsem-mcp"
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing; restore mock server"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    mock_proc = None
    session_id = vm_name("ironbank-mcp")
    try:
        service.start()
        client = service.client()
        mock_proc, ready = start_mock_server()
        url = f"{ready['base_url']}/html/about"

        created = client.post(
            "/vms/create",
            {
                "name": session_id,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
            timeout=90,
        )
        assert created is not None
        assert created.get("id") == session_id or created.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

        with _mcp_session(service.uds_path) as mcp:
            route_servers = client.get(
                f"/profiles/{CODE_PROFILE_ID}/mcp/servers/list",
                timeout=30,
            )
            assert isinstance(route_servers, list)
            assert route_servers
            assert all(set(server) == EXPECTED_MCP_SERVER_FIELDS for server in route_servers)
            local_route_server = next(server for server in route_servers if server["name"] == "local")
            assert local_route_server["enabled"] is True
            assert local_route_server["is_stdio"] is True
            assert local_route_server["source"] == "builtin"
            assert local_route_server["tool_count"] >= 3

            route_tools = client.get(
                f"/profiles/{CODE_PROFILE_ID}/mcp/servers/local/tools/list",
                timeout=30,
            )
            assert isinstance(route_tools, list)
            assert route_tools
            assert all(set(tool) == EXPECTED_MCP_TOOL_FIELDS for tool in route_tools)
            route_http_tool = next(
                tool for tool in route_tools if tool["namespaced_name"] == "local__http_headers"
            )
            assert route_http_tool["original_name"] == "http_headers"
            assert route_http_tool["server_name"] == "local"
            assert route_http_tool["permission_action"] in {"allow", "ask"}
            assert route_http_tool["permission_source"]
            assert route_http_tool["pin_changed"] is False

            mcp_servers = _json_tool_result(mcp.call_tool("capsem_mcp_servers"))
            assert isinstance(mcp_servers, list)
            assert any(server["name"] == "local" for server in mcp_servers)

            mcp_tools = _json_tool_result(mcp.call_tool("capsem_mcp_tools", {"server": "local"}))
            assert isinstance(mcp_tools, list)
            mcp_http_tool = next(
                tool for tool in mcp_tools if tool["namespaced_name"] == "local__http_headers"
            )
            assert mcp_http_tool == route_http_tool

            with _connect_session_db(service.tmp_dir / "sessions", session_id) as conn:
                before_count = conn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0]

            call_envelope = _json_tool_result(
                mcp.call_tool(
                    "capsem_mcp_call",
                    {
                        "name": "local__http_headers",
                        "arguments": {"url": url, "method": "GET"},
                    },
                )
            )
            assert call_envelope["jsonrpc"] == "2.0"
            assert "error" not in call_envelope
            assert call_envelope["result"]["content"][0]["type"] == "text"
            call_text = call_envelope["result"]["content"][0]["text"]
            assert "Status: 200 OK" in call_text
            assert "content-type:" in call_text.lower()

        with _connect_session_db(service.tmp_dir / "sessions", session_id) as conn:
            mcp_rows = _eventually(
                lambda: _rows(
                    conn,
                    """
                    SELECT event_id, server_name, method, tool_name, decision,
                           bytes_sent, bytes_received, request_preview,
                           response_preview, trace_id
                    FROM mcp_calls
                    WHERE method = 'tools/call'
                      AND tool_name IN ('http_headers', 'local__http_headers')
                    ORDER BY id DESC
                    LIMIT 1
                    """,
                ),
                lambda rows: len(rows) == 1,
            )
            mcp_row = mcp_rows[0]
            assert conn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0] == before_count + 1
            _assert_event_id(mcp_row["event_id"])
            assert mcp_row["server_name"] == "local"
            assert mcp_row["method"] == "tools/call"
            assert mcp_row["tool_name"] in {"http_headers", "local__http_headers"}
            assert mcp_row["decision"] == "allowed"
            assert mcp_row["bytes_sent"] > 0
            assert mcp_row["bytes_received"] > 0
            assert "local__http_headers" in mcp_row["request_preview"]
            assert "Status: 200 OK" in mcp_row["response_preview"]
            assert mcp_row["trace_id"]

            net_rows = _rows(
                conn,
                """
                SELECT event_id, domain, method, path, status_code, decision,
                       conn_type, bytes_received
                FROM net_events
                WHERE conn_type = 'mcp_builtin'
                  AND path = '/html/about'
                ORDER BY id DESC
                LIMIT 1
                """,
            )
            assert len(net_rows) == 1
            net_row = net_rows[0]
            _assert_event_id(net_row["event_id"])
            assert net_row["domain"] == "127.0.0.1"
            assert net_row["method"] == "GET"
            assert net_row["status_code"] == 200
            assert net_row["decision"] == "allowed"
            assert net_row["bytes_received"] > 0

            security_rows = _rows(
                conn,
                """
                SELECT event_type, rule_id, rule_action, detection_level,
                       event_json, rule_json, trace_id
                FROM security_rule_events
                WHERE event_id = ?
                ORDER BY id
                """,
                (mcp_row["event_id"],),
            )
            assert security_rows
            assert any(row["event_type"] == "mcp.tool_call" for row in security_rows)
            assert any(row["rule_id"] == "profiles.rules.default_mcp" for row in security_rows)
            assert {row["rule_action"] for row in security_rows} <= {"allow", "ask"}
            assert all(
                row["detection_level"] in {"none", "informational"} for row in security_rows
            )
            assert all(row["trace_id"] == mcp_row["trace_id"] for row in security_rows)
            for row in security_rows:
                event = json.loads(row["event_json"])
                rule = json.loads(row["rule_json"])
                assert event["event_type"] == "mcp.tool_call"
                assert event["mcp"]["server_name"] == "local"
                assert event["mcp"]["tool_call_name"] in {"http_headers", "local__http_headers"}
                assert rule["name"]
    finally:
        if mock_proc is not None:
            stop_process(mock_proc)
        try:
            service.client().delete(f"/vms/{session_id}/delete", timeout=30)
        except Exception:
            pass
        service.stop()
