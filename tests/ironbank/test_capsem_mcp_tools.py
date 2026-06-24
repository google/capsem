"""Ironbank black-box proof for the host capsem-mcp tool surface."""

from __future__ import annotations

from contextlib import closing, contextmanager
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
import uuid
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

EXPECTED_CAPSEM_MCP_TOOLS = {
    "capsem_create",
    "capsem_delete",
    "capsem_exec",
    "capsem_fork",
    "capsem_host_logs",
    "capsem_info",
    "capsem_list",
    "capsem_mcp_call",
    "capsem_mcp_servers",
    "capsem_mcp_tools",
    "capsem_panics",
    "capsem_persist",
    "capsem_purge",
    "capsem_read_file",
    "capsem_resume",
    "capsem_run",
    "capsem_service_logs",
    "capsem_stop",
    "capsem_suspend",
    "capsem_timeline",
    "capsem_triage",
    "capsem_version",
    "capsem_vm_logs",
    "capsem_write_file",
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
        self.proc.stdin.write(json.dumps(req, separators=(",", ":")) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        assert line, "capsem-mcp closed stdout"
        return json.loads(line)

    def notify(self, method: str, params: dict | None = None) -> None:
        req = {"jsonrpc": "2.0", "method": method, "params": params or {}}
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(req, separators=(",", ":")) + "\n")
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
    env["RUST_LOG"] = "service=info,capsem_mcp=info,info"
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
    init = session.request(
        "initialize",
        {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "ironbank-capsem-mcp", "version": "1.0"},
        },
    )
    assert init["result"]["serverInfo"]["name"] == "capsem-mcp"
    session.notify("notifications/initialized")
    try:
        yield session
    finally:
        kill_mcp_proc(proc)


def _json_tool_result(result: dict) -> object:
    return json.loads(content_text(result))


def _rows(conn: sqlite3.Connection, sql: str, params: tuple = ()) -> list[sqlite3.Row]:
    return conn.execute(sql, params).fetchall()


def _eventually(query, predicate, timeout: float = 20.0):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = query()
        if predicate(last):
            return last
        time.sleep(0.25)
    assert predicate(last), f"condition not met before timeout; last={last!r}"
    return last


def _connect_session_db(service: ServiceInstance, session_id: str):
    candidates = [
        service.tmp_dir / "sessions" / session_id / "session.db",
        service.tmp_dir / "persistent" / session_id / "session.db",
    ]
    db_path = next((path for path in candidates if path.exists()), candidates[0])
    assert db_path.exists(), f"session DB missing at {db_path}"
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def _assert_event_id(value: object) -> None:
    assert isinstance(value, str)
    assert re.fullmatch(r"[0-9a-f]{12}", value), value


def _assert_success_payload(payload: object) -> dict:
    assert isinstance(payload, dict), payload
    assert payload.get("success") is True, payload
    return payload


def _close_mcp_proc_gracefully(proc: subprocess.Popen[str]) -> None:
    assert proc.stdin is not None
    proc.stdin.close()
    proc.wait(timeout=5)
    for pipe in (proc.stdout, proc.stderr):
        if pipe is not None and not pipe.closed:
            pipe.close()


def test_capsem_mcp_tools_pay_exact_host_and_session_ledger_blackbox():
    assert MCP_BINARY.exists(), f"{MCP_BINARY} missing; build capsem-mcp"
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing; restore mock server"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    mock_proc = None
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    session_id = vm_name("ironbank-capsem-mcp")
    fork_id = f"{session_id}-fork"
    nonce = uuid.uuid4().hex
    guest_path = f"/root/ironbank-capsem-mcp-{nonce[:8]}.txt"
    expected_file = f"capsem-mcp ledger {nonce}\n"
    try:
        corp_path = service.tmp_dir / "ironbank-capsem-mcp-corp.toml"
        corp_path.write_text(
            """
[corp.rules.allow_capsem_mcp_mock_http]
name = "allow_capsem_mcp_mock_http"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Allow hermetic Ironbank capsem-mcp builtin HTTP tool calls."
match = 'http.host == "127.0.0.1" && tcp.port == "3713"'
""".lstrip(),
            encoding="utf-8",
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp_path)
        service.start()
        client = service.client()
        mock_proc, ready = start_mock_server()
        url = f"{ready['base_url']}/html/about"

        with _mcp_session(service.uds_path) as mcp:
            listed = mcp.request("tools/list")
            tool_names = {tool["name"] for tool in listed["result"]["tools"]}
            assert tool_names == EXPECTED_CAPSEM_MCP_TOOLS
            for tool in listed["result"]["tools"]:
                assert set(tool) >= {"name", "description", "inputSchema"}
                assert isinstance(tool["description"], str) and tool["description"]
                assert tool["inputSchema"]["type"] == "object"

            version = _json_tool_result(mcp.call_tool("capsem_version"))
            assert set(version) == {"mcp_version", "service"}
            assert version["service"] == "connected"
            assert re.fullmatch(r"\d+\.\d+\.\d+.*", version["mcp_version"])

            listed_sessions = _json_tool_result(mcp.call_tool("capsem_list"))
            assert set(listed_sessions) >= {"sandboxes"}
            assert listed_sessions["sandboxes"] == []
            panics = _json_tool_result(mcp.call_tool("capsem_panics"))
            assert panics == {"panics": []}
            triage = _json_tool_result(mcp.call_tool("capsem_triage"))
            assert set(triage) == {"host", "rank", "session", "session_id", "since"}
            assert set(triage["host"]) == {"errors", "panics", "slow_ops"}
            assert isinstance(triage["rank"], list)
            service_logs = content_text(mcp.call_tool("capsem_service_logs", {"tail": 20}))
            assert "service-logs" in service_logs
            assert '"level"' in service_logs
            host_logs = content_text(
                mcp.call_tool("capsem_host_logs", {"name": "service", "tail": 20})
            )
            assert "service-logs" in host_logs
            assert '"level"' in host_logs

            created = _json_tool_result(
                mcp.call_tool(
                    "capsem_create",
                    {"name": session_id, "ramMb": DEFAULT_RAM_MB, "cpuCount": DEFAULT_CPUS},
                )
            )
            assert created.get("id") == session_id or created.get("name") == session_id
            assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)

            info = _json_tool_result(mcp.call_tool("capsem_info", {"id": session_id}))
            assert info["id"] == session_id or info["name"] == session_id
            assert info["profile_id"] == CODE_PROFILE_ID
            assert info["status"] in {"Running", "running", "ready"}

            write_payload = _assert_success_payload(
                _json_tool_result(
                    mcp.call_tool(
                        "capsem_write_file",
                        {"id": session_id, "path": guest_path, "content": expected_file},
                    )
                )
            )
            assert set(write_payload) >= {"success"}
            read_payload = _json_tool_result(
                mcp.call_tool("capsem_read_file", {"id": session_id, "path": guest_path})
            )
            assert read_payload["content"] == expected_file

            exec_payload = _json_tool_result(
                mcp.call_tool(
                    "capsem_exec",
                    {"id": session_id, "command": f"printf {nonce!r}", "timeout": 30},
                )
            )
            assert exec_payload["exit_code"] == 0
            assert exec_payload["stdout"] == nonce
            assert exec_payload["stderr"] == ""

            route_servers = client.get(
                f"/profiles/{CODE_PROFILE_ID}/mcp/servers/list",
                timeout=30,
            )
            mcp_servers = _json_tool_result(mcp.call_tool("capsem_mcp_servers"))
            assert mcp_servers == route_servers
            assert any(server["name"] == "local" for server in mcp_servers)

            route_tools = client.get(
                f"/profiles/{CODE_PROFILE_ID}/mcp/servers/local/tools/list",
                timeout=30,
            )
            mcp_tools = _json_tool_result(mcp.call_tool("capsem_mcp_tools", {"server": "local"}))
            assert mcp_tools == route_tools
            assert {tool["namespaced_name"] for tool in mcp_tools} >= {
                "local__http_headers",
                "local__fetch_http",
            }

            with closing(_connect_session_db(service, session_id)) as conn:
                before_protocol_rows = conn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0]
                before_tool_rows = conn.execute(
                    "SELECT COUNT(*) FROM tool_calls WHERE origin = 'mcp'"
                ).fetchone()[0]

            call_payload = _json_tool_result(
                mcp.call_tool(
                    "capsem_mcp_call",
                    {
                        "name": "local__http_headers",
                        "arguments": {"url": url, "method": "GET"},
                    },
                )
            )
            assert call_payload["jsonrpc"] == "2.0"
            assert "error" not in call_payload
            call_text = call_payload["result"]["content"][0]["text"]
            assert "Status: 200 OK" in call_text
            assert "content-type:" in call_text.lower()

            timeline = _json_tool_result(
                mcp.call_tool(
                    "capsem_timeline",
                    {"id": session_id, "layers": "exec,fs,mcp", "limit": 50},
                )
            )
            assert set(timeline) == {"columns", "rows"}
            assert {"layer", "summary", "status"} <= set(timeline["columns"])
            timeline_rows = [dict(zip(timeline["columns"], row, strict=True)) for row in timeline["rows"]]
            assert any(row["layer"] == "exec" and nonce in row["summary"] for row in timeline_rows)
            assert any(row["layer"] == "fs" and guest_path in row["summary"] for row in timeline_rows)
            assert any(
                row["layer"] == "mcp" and "http_headers" in row["summary"]
                for row in timeline_rows
            )

            vm_logs = content_text(mcp.call_tool("capsem_vm_logs", {"id": session_id, "tail": 50}))
            assert isinstance(vm_logs, str)

            fork_payload = _json_tool_result(
                mcp.call_tool(
                    "capsem_fork",
                    {"id": session_id, "name": fork_id, "description": "Ironbank MCP fork proof"},
                )
            )
            assert fork_payload.get("id") == fork_id or fork_payload.get("name") == fork_id
            fork_info = _json_tool_result(mcp.call_tool("capsem_info", {"id": fork_id}))
            assert fork_info["status"] in {"Stopped", "stopped", "paused", "created"}

            with closing(_connect_session_db(service, session_id)) as conn:
                tool_rows = _eventually(
                    lambda: _rows(
                        conn,
                        """
                        SELECT event_id, server_name, method, tool_name, decision,
                               bytes_sent, bytes_received, arguments AS request_preview,
                               response_preview, trace_id
                        FROM tool_calls
                        WHERE origin = 'mcp'
                        ORDER BY id
                        """,
                    ),
                    lambda rows: len(rows) == before_tool_rows + 1,
                )
                assert len(tool_rows) == before_tool_rows + 1
                assert conn.execute("SELECT COUNT(*) FROM mcp_calls").fetchone()[0] == before_protocol_rows
                tool_row = tool_rows[-1]
                _assert_event_id(tool_row["event_id"])
                assert tool_row["server_name"] == "local"
                assert tool_row["method"] == "tools/call"
                assert tool_row["tool_name"] in {"http_headers", "local__http_headers"}
                assert tool_row["decision"] == "allowed"
                assert tool_row["bytes_sent"] > 0
                assert tool_row["bytes_received"] > 0
                assert "local__http_headers" in tool_row["request_preview"]
                assert "Status: 200 OK" in tool_row["response_preview"]
                assert tool_row["trace_id"]

                fs_rows = _rows(
                    conn,
                    """
                    SELECT event_id, action, path, size, trace_id
                    FROM fs_events
                    WHERE path = ?
                    ORDER BY id
                    """,
                    (guest_path.lstrip("/root/"),),
                )
                assert fs_rows
                assert {row["action"] for row in fs_rows} == {"created"}
                assert any(row["size"] == len(expected_file.encode()) for row in fs_rows)
                for row in fs_rows:
                    _assert_event_id(row["event_id"])
                    assert row["trace_id"]

                exec_rows = _rows(
                    conn,
                    """
                    SELECT event_id, command, exit_code, stdout_preview, stderr_preview,
                           stdout_bytes, stderr_bytes, source, trace_id
                    FROM exec_events
                    WHERE command LIKE ?
                    ORDER BY id
                    """,
                    (f"%{nonce}%",),
                )
                assert len(exec_rows) == 1
                exec_row = exec_rows[0]
                _assert_event_id(exec_row["event_id"])
                assert exec_row["exit_code"] == 0
                assert exec_row["stdout_preview"] == nonce
                assert exec_row["stderr_preview"] in {None, ""}
                assert exec_row["stdout_bytes"] == len(nonce.encode())
                assert exec_row["stderr_bytes"] == 0
                assert exec_row["source"] == "api"
                assert exec_row["trace_id"]

                snapshot_tables = [
                    row[0]
                    for row in conn.execute(
                        "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE '%snapshot%'"
                    ).fetchall()
                ]
                for table in snapshot_tables:
                    count = conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                    assert count == 0, f"phantom snapshot rows in {table}"

                security_rows = _rows(
                    conn,
                    """
                    SELECT event_type, rule_id, rule_action, detection_level,
                           event_json, rule_json, trace_id
                    FROM security_rule_events
                    WHERE event_id = ?
                    ORDER BY id
                    """,
                    (tool_row["event_id"],),
                )
                assert security_rows
                assert {row["event_type"] for row in security_rows} == {"mcp.tool_call"}
                assert any(row["rule_id"] == "profiles.rules.default_mcp" for row in security_rows)
                assert all(row["rule_action"] in {"allow", "ask"} for row in security_rows)
                assert all(
                    row["detection_level"] in {"none", "informational"}
                    for row in security_rows
                )
                for row in security_rows:
                    assert row["trace_id"] == tool_row["trace_id"]
                    event = json.loads(row["event_json"])
                    rule = json.loads(row["rule_json"])
                    assert event["event_type"] == "mcp.tool_call"
                    assert event["mcp"]["server_name"] == "local"
                    assert event["mcp"]["tool_call_name"] in {
                        "http_headers",
                        "local__http_headers",
                    }
                    assert rule["name"]

            security_latest = client.get(f"/vms/{session_id}/security/latest?limit=100", timeout=30)
            assert any(row["event_type"] == "mcp.tool_call" for row in security_latest)
            security_status = client.get(f"/vms/{session_id}/security/status", timeout=30)
            by_event_type = {
                row["event_type"]: row["count"] for row in security_status["by_event_type"]
            }
            assert by_event_type["mcp.tool_call"] >= 1

            deleted_fork = _assert_success_payload(
                _json_tool_result(mcp.call_tool("capsem_delete", {"id": fork_id}))
            )
            assert set(deleted_fork) >= {"success"}
            stopped = _json_tool_result(mcp.call_tool("capsem_stop", {"id": session_id}))
            assert stopped.get("id") == session_id or stopped.get("success") is True
            resumed = _json_tool_result(mcp.call_tool("capsem_resume", {"name": session_id}))
            assert resumed.get("id") == session_id or resumed.get("name") == session_id
            purged = _json_tool_result(mcp.call_tool("capsem_purge", {"all": False}))
            assert isinstance(purged, dict)

            service_log = (service.tmp_dir / "service.log").read_text(encoding="utf-8")
            assert "profile_mcp_tool_call" in service_log or "mcp" in service_log.lower()
            _close_mcp_proc_gracefully(mcp.proc)
            mcp_log = (service.tmp_dir / "mcp.log").read_text(encoding="utf-8")
            assert "capsem-mcp starting" in mcp_log
            assert "Registered" in mcp_log
    finally:
        if old_corp_config is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp_config
        if mock_proc is not None:
            stop_process(mock_proc)
        try:
            service.client().delete(f"/vms/{fork_id}/delete", timeout=30)
        except Exception:
            pass
        try:
            service.client().delete(f"/vms/{session_id}/delete", timeout=30)
        except Exception:
            pass
        service.stop()
