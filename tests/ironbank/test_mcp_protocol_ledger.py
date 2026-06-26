"""Ironbank black-box observed MCP protocol ledger tests."""

from __future__ import annotations

from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import textwrap
import time
import uuid

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.mock_server import MOCK_SERVER_BINARY, start_mock_server, stop_process
from helpers.service import ServiceInstance, vm_session_db_path, wait_exec_ready, vm_name

pytestmark = pytest.mark.integration

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ASSETS_DIR = PROJECT_ROOT / "assets"
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"

EXPECTED_SECURITY_COLUMNS = {
    "id",
    "timestamp_unix_ms",
    "event_id",
    "event_type",
    "rule_id",
    "rule_action",
    "detection_level",
    "rule_json",
    "event_json",
    "trace_id",
    "turn_id",
    "credential_ref",
}


def _connect_session_db(service: ServiceInstance, session_id: str) -> sqlite3.Connection:
    db_path = vm_session_db_path(service.tmp_dir, service.client(), session_id)
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()}


def _query_rows(client, session_id: str, sql: str) -> list[dict]:
    db_path = vm_session_db_path(Path(client.socket_path).parent, client, session_id)
    with closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        return [dict(row) for row in conn.execute(sql).fetchall()]


def _event_id(value: object) -> str:
    assert isinstance(value, str)
    assert len(value) == 12
    assert all(ch in "0123456789abcdef" for ch in value)
    return value


def _eventually(fetch, predicate, *, timeout_s: float = 20.0, interval_s: float = 0.25):
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        last = fetch()
        if predicate(last):
            return last
        time.sleep(interval_s)
    assert predicate(last), f"condition not met before timeout; last={last!r}"
    return last


def _one_json_line(stdout: str, prefix: str) -> dict:
    line = next((line for line in stdout.splitlines() if line.startswith(prefix)), None)
    assert line is not None, stdout
    return json.loads(line.split("=", 1)[1])


def _mcp_probe_script(base_url: str, nonce: str) -> str:
    payload = {"url": f"{base_url.rstrip('/')}/mcp", "nonce": nonce}
    return textwrap.dedent(
        f"""
        import json
        import urllib.request

        cfg = json.loads({json.dumps(json.dumps(payload))})

        def call_mcp(body):
            request = urllib.request.Request(
                cfg["url"],
                data=json.dumps(body, separators=(",", ":")).encode("utf-8"),
                headers={{"Content-Type": "application/json"}},
                method="POST",
            )
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.loads(response.read().decode("utf-8"))

        initialize = call_mcp({{
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {{"clientInfo": {{"name": "ironbank-mcp", "version": "1.0"}}}},
        }})
        tools = call_mcp({{"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {{}}}})
        tool = call_mcp({{
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {{"name": "fixture_lookup", "arguments": {{"query": cfg["nonce"]}}}},
        }})
        tool_names = [item["name"] for item in tools["result"]["tools"]]
        result = {{
            "initialize_server": initialize["result"]["serverInfo"]["name"],
            "initialize_version": initialize["result"]["serverInfo"]["version"],
            "tool_count": len(tool_names),
            "has_fixture_lookup": "fixture_lookup" in tool_names,
            "has_fetch_http": "fetch_http" in tool_names,
            "tool_text": tool["result"]["content"][0]["text"],
            "tool_is_error": tool["result"].get("isError"),
        }}
        print("IRONBANK_MCP_PROTOCOL_RESULT=" + json.dumps(result, sort_keys=True))
        """
    ).strip()


def _read_jsonl(path: str | Path) -> list[dict]:
    file_path = Path(path)
    assert file_path.exists(), f"mock request log missing at {file_path}"
    return [
        json.loads(line)
        for line in file_path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def test_observed_remote_mcp_protocol_pays_full_ledger_blackbox():
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing; restore mock server"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    gateway = None
    gateway_client = None
    mock_proc = None
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    session_id = vm_name("ironbank-mcp-protocol")
    vm_id: str | None = None
    nonce = uuid.uuid4().hex[:12]
    try:
        corp_path = service.tmp_dir / "ironbank-corp.toml"
        corp_path.write_text(
            textwrap.dedent(
                """
                [corp.rules.allow_ironbank_mock_mcp_server]
                name = "allow_ironbank_mock_mcp_server"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow the hermetic Ironbank observed MCP fixture."
                match = 'mcp.server.name == "observed:127.0.0.1:3713/mcp" || (ip.value == "127.0.0.1" && tcp.port == "3713")'
                """
            ).strip()
            + "\n",
            encoding="utf-8",
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp_path)

        service.start()
        client = service.client()
        gateway = GatewayInstance(service.uds_path)
        gateway.start()
        gateway_client = TcpHttpClient(gateway.base_url, gateway.token)
        mock_proc, ready = start_mock_server()
        mock_base_url = ready["base_url"]
        observed_server = "observed:127.0.0.1:3713/mcp"

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
        vm_id = created["id"]
        assert isinstance(vm_id, str)
        assert created.get("name") == session_id
        assert wait_exec_ready(client, vm_id, timeout=EXEC_READY_TIMEOUT)

        script_name = f"ironbank-mcp-protocol-{uuid.uuid4().hex[:8]}.py"
        script = _mcp_probe_script(mock_base_url, nonce).encode()
        upload = client.post_bytes(
            f"/vms/{vm_id}/files/content?path={script_name}",
            script,
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True
        assert upload["size"] == len(script)
        exec_resp = client.post(
            f"/vms/{vm_id}/exec",
            {"command": f"python3 /root/{script_name}", "timeout_secs": 120},
            timeout=150,
        )
        assert exec_resp is not None, "MCP protocol exec returned no body"
        assert exec_resp["exit_code"] == 0, exec_resp
        result = _one_json_line(
            exec_resp.get("stdout") or "",
            "IRONBANK_MCP_PROTOCOL_RESULT=",
        )
        assert result == {
            "has_fetch_http": True,
            "has_fixture_lookup": True,
            "initialize_server": "capsem-mock-server",
            "initialize_version": "1.0.0",
            "tool_count": 3,
            "tool_is_error": False,
            "tool_text": "capsem-mock-server:mcp:fixture_lookup",
        }

        upstream_records = _eventually(
            lambda: [row for row in _read_jsonl(ready["request_log"]) if row["path"] == "/mcp"],
            lambda rows: len(rows) >= 3,
        )
        upstream_bodies = [json.loads(row["request_body"]) for row in upstream_records]
        assert [body["method"] for body in upstream_bodies[:3]] == [
            "initialize",
            "tools/list",
            "tools/call",
        ]
        assert upstream_bodies[2]["params"] == {
            "name": "fixture_lookup",
            "arguments": {"query": nonce},
        }
        assert all(row["status"] == 200 for row in upstream_records[:3])

        with closing(_connect_session_db(service, vm_id)) as conn:
            assert "mcp_calls" not in {
                row["name"]
                for row in conn.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                ).fetchall()
            }
            assert _table_columns(conn, "security_rule_events") == EXPECTED_SECURITY_COLUMNS

            call_row = _eventually(
                lambda: conn.execute(
                    """
                    SELECT *
                    FROM tool_calls
                    WHERE origin = 'mcp'
                      AND server_name = ?
                      AND method = 'tools/call'
                      AND tool_name = 'fixture_lookup'
                    ORDER BY id DESC
                    """,
                    (observed_server,),
                ).fetchall(),
                lambda rows: len(rows) == 1,
            )[0]
            trace_id = call_row["trace_id"]
            assert trace_id
            _event_id(call_row["event_id"])
            assert call_row["model_call_id"] is None
            assert call_row["provider"] == ""
            assert call_row["status"] == "responded"
            assert call_row["tool_name"] == "fixture_lookup"
            assert call_row["decision"] == "allowed"
            assert call_row["duration_ms"] >= 0
            assert call_row["error_message"] is None
            assert call_row["bytes_sent"] > 0
            assert call_row["bytes_received"] > 0
            assert call_row["policy_action"] == "allow"
            assert call_row["policy_rule"] == "corp.rules.allow_ironbank_mock_mcp_server"
            assert call_row["trace_id"] == trace_id
            assert call_row["credential_ref"] is None

            call_request = json.loads(call_row["arguments"])
            call_response = json.loads(call_row["response_preview"])
            assert call_request == {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {"name": "fixture_lookup", "arguments": {"query": nonce}},
            }
            assert call_response["result"]["content"] == [
                {"type": "text", "text": "capsem-mock-server:mcp:fixture_lookup"}
            ]
            assert call_response["result"]["isError"] is False

            security_rows = conn.execute(
                """
                SELECT *
                FROM security_rule_events
                WHERE event_id = ?
                ORDER BY id
                """,
                (call_row["event_id"],),
            ).fetchall()
            assert security_rows
            security_by_event: dict[str, list[sqlite3.Row]] = {}
            for row in security_rows:
                security_by_event.setdefault(row["event_id"], []).append(row)
                assert row["trace_id"] == trace_id
                assert json.loads(row["rule_json"])["name"]
                event = json.loads(row["event_json"])
                assert event["mcp"]["server_name"] == observed_server
                assert event["tcp"]["port"] == "3713"
                assert event["ip"]["value"] == "127.0.0.1"

            list_security = conn.execute(
                """
                SELECT *
                FROM security_rule_events
                WHERE event_type = 'mcp.tool_list'
                ORDER BY id
                """
            ).fetchall()
            assert list_security
            assert {row["event_type"] for row in list_security} == {"mcp.tool_list"}
            assert {row["rule_id"] for row in list_security} >= {
                "corp.rules.allow_ironbank_mock_mcp_server",
                "profiles.rules.default_mcp",
            }
            assert any(row["detection_level"] == "informational" for row in list_security)
            for row in list_security:
                assert row["trace_id"]
                assert json.loads(row["rule_json"])["name"]
            list_event = json.loads(list_security[0]["event_json"])
            assert list_event["event_type"] == "mcp.tool_list"
            assert list_event["mcp"]["method"] == "tools/list"
            listed_tools = json.loads(list_event["mcp"]["tool_list"])["result"]["tools"]
            assert len(listed_tools) == 3
            assert {tool["name"] for tool in listed_tools} >= {
                "fixture_lookup",
                "fetch_http",
            }

            call_security = security_by_event[call_row["event_id"]]
            assert {row["event_type"] for row in call_security} == {"mcp.tool_call"}
            assert {row["rule_id"] for row in call_security} >= {
                "corp.rules.allow_ironbank_mock_mcp_server",
                "profiles.rules.default_mcp",
            }
            call_actions = {row["rule_action"] for row in call_security}
            assert "allow" in call_actions
            assert any(
                row["rule_id"] == "corp.rules.allow_ironbank_mock_mcp_server"
                and row["rule_action"] == "allow"
                for row in call_security
            )
            assert any(
                row["rule_id"] == "profiles.rules.default_000_local_network"
                and row["rule_action"] == "ask"
                for row in call_security
            )
            call_event = json.loads(call_security[0]["event_json"])
            assert call_event["event_type"] == "mcp.tool_call"
            assert call_event["mcp"]["method"] == "tools/call"
            assert call_event["mcp"]["tool_call_name"] == "fixture_lookup"
            assert call_event["mcp"]["request"]["arguments"] == {"query": nonce}
            assert call_event["mcp"]["response"]["content"] == [
                {"type": "text", "text": "capsem-mock-server:mcp:fixture_lookup"}
            ]

            mcp_tool_count = conn.execute(
                "SELECT COUNT(*) FROM tool_calls WHERE origin = 'mcp' AND tool_name = 'fixture_lookup'"
            ).fetchone()[0]
            model_tool_response_count = conn.execute(
                "SELECT COUNT(*) FROM tool_responses"
            ).fetchone()[0]
            assert mcp_tool_count == 1
            assert model_tool_response_count == 0

        uds_tool_rows = _query_rows(
            client,
            vm_id,
            f"""
            SELECT event_id, server_name, method, tool_name, decision,
                   policy_action, policy_rule, trace_id
            FROM tool_calls
            WHERE origin = 'mcp' AND server_name = '{observed_server}'
            ORDER BY id
            """,
        )
        assert len(uds_tool_rows) == 1
        assert uds_tool_rows[0]["method"] == "tools/call"
        assert uds_tool_rows[0]["tool_name"] == "fixture_lookup"
        assert uds_tool_rows[0]["policy_rule"] == "corp.rules.allow_ironbank_mock_mcp_server"

        timeline = _eventually(
            lambda: client.get(
                f"/vms/{vm_id}/timeline?layers=tool&limit=50",
                timeout=30,
            ),
            lambda payload: any(
                row["summary"].startswith(f"{observed_server}/fixture_lookup")
                for row in [
                    dict(zip(payload["columns"], row, strict=True))
                    for row in payload["rows"]
                ]
            ),
        )
        assert set(timeline) == {"columns", "rows"}
        assert {"timestamp", "layer", "ref", "summary", "status", "duration_ms"} <= set(
            timeline["columns"]
        )
        timeline_rows = [
            dict(zip(timeline["columns"], row, strict=True)) for row in timeline["rows"]
        ]
        timeline_summaries = {row["summary"] for row in timeline_rows}
        assert any(
            summary.startswith(f"{observed_server}/fixture_lookup")
            for summary in timeline_summaries
        )

        security_latest = _eventually(
            lambda: client.get(f"/vms/{vm_id}/security/latest?limit=100", timeout=30),
            lambda rows: {row["event_id"] for row in uds_tool_rows}
            <= {row["event_id"] for row in rows},
        )
        assert isinstance(security_latest, list)
        latest_ids = {row["event_id"] for row in security_latest}
        assert {row["event_id"] for row in uds_tool_rows} <= latest_ids
        latest_call_rows = [row for row in security_latest if row["event_id"] == uds_tool_rows[0]["event_id"]]
        assert any(row["event_type"] == "mcp.tool_call" for row in latest_call_rows)
        assert any(
            row["rule_id"] == "corp.rules.allow_ironbank_mock_mcp_server"
            and row["detection_level"] == "informational"
            for row in latest_call_rows
        )

        gateway_latest = gateway_client.get(
            f"/vms/{vm_id}/security/latest?limit=100",
            timeout=30,
        )
        assert gateway_latest == security_latest

        security_status = client.get(f"/vms/{vm_id}/security/status", timeout=30)
        by_action = {row["rule_action"]: row["count"] for row in security_status["by_action"]}
        by_event_type = {
            row["event_type"]: row["count"] for row in security_status["by_event_type"]
        }
        by_level = {row["detection_level"]: row["count"] for row in security_status["by_level"]}
        assert by_action["allow"] >= 3
        assert by_event_type["mcp.tool_call"] >= 1
        assert by_event_type["mcp.tool_list"] >= 1
        assert by_level["informational"] >= 2

        info = _eventually(
            lambda: client.get(f"/vms/{vm_id}/info", timeout=30),
            lambda value: (
                value is not None
                and value.get("id") == vm_id
                and value.get("name") == session_id
                and value.get("session_db", {}).get("ready") is True
            ),
            timeout_s=20,
        )
        assert info["profile_id"] == CODE_PROFILE_ID
        assert info.get("total_tool_calls") is None
        stats_detail = client.get(f"/vms/{vm_id}/stats/detail", timeout=30)
        assert isinstance(stats_detail, dict)
        mcp_tool_events = [
            row
            for row in stats_detail.get("tool_events", [])
            if row["source"] == "mcp" and row["method"] == "tools/call"
        ]
        assert len(mcp_tool_events) == 1
        assert mcp_tool_events[0]["tool_name"] == "fixture_lookup"

        service_log = (service.tmp_dir / "service.log").read_text(encoding="utf-8")
        gateway_log = gateway.stop_and_read_log()
        assert "gateway.proxy.ok" in gateway_log
        assert "security_latest" in service_log or "/security/latest" in service_log
        assert "mcp" in service_log.lower()
    finally:
        if old_corp_config is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp_config
        if mock_proc is not None:
            stop_process(mock_proc)
        if gateway is not None:
            gateway.stop()
        try:
            assert vm_id is not None
            service.client().delete(f"/vms/{vm_id}/delete", timeout=30)
        except Exception:
            pass
        service.stop()
