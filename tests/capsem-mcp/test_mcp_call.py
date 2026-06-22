"""capsem_mcp_call: route tool invocations through a running VM's aggregator."""

import json
import uuid

import pytest

from helpers.mcp import content_text
from helpers.mock_server import start_mock_server, stop_process

pytestmark = pytest.mark.mcp


def _json_tool_result(result):
    return json.loads(content_text(result))


def _inspect(mcp_session, vm_name, sql):
    result = mcp_session.call_tool("capsem_inspect", {"id": vm_name, "sql": sql})
    payload = json.loads(content_text(result))
    return [dict(zip(payload["columns"], row, strict=True)) for row in payload["rows"]]


def test_mcp_call_builtin_http_headers_pays_full_ledger(shared_vm, mcp_session):
    """Host MCP -> service profile route -> VM aggregator -> DB/security ledger."""
    vm_name, _ = shared_vm
    mock_proc, ready = start_mock_server()
    try:
        before_count = _inspect(
            mcp_session,
            vm_name,
            "SELECT COUNT(*) AS count FROM tool_calls WHERE origin = 'mcp'",
        )[0]["count"]

        servers = _json_tool_result(mcp_session.call_tool("capsem_mcp_servers"))
        local_server = next(server for server in servers if server["name"] == "local")
        assert local_server["enabled"] is True
        assert local_server["is_stdio"] is True
        assert local_server["tool_count"] >= 3

        tools = _json_tool_result(
            mcp_session.call_tool("capsem_mcp_tools", {"server": "local"})
        )
        by_name = {tool["namespaced_name"]: tool for tool in tools}
        http_headers = by_name["local__http_headers"]
        assert http_headers["server_name"] == "local"
        assert http_headers["original_name"] == "http_headers"
        assert http_headers["permission_action"] in {"allow", "ask"}
        assert http_headers["permission_source"]
        assert http_headers["pin_changed"] is False

        after_list_count = _inspect(
            mcp_session,
            vm_name,
            "SELECT COUNT(*) AS count FROM tool_calls WHERE origin = 'mcp'",
        )[0]["count"]
        assert after_list_count == before_count, "tool listing must not emit phantom calls"

        url = f"{ready['base_url']}/html/about"
        call_envelope = _json_tool_result(
            mcp_session.call_tool(
                "capsem_mcp_call",
                {
                    "name": "local__http_headers",
                    "arguments": {"url": url, "method": "GET"},
                },
            )
        )
        assert call_envelope["jsonrpc"] == "2.0"
        assert "error" not in call_envelope
        call_payload = call_envelope["result"]
        assert call_payload["content"][0]["type"] == "text"
        call_text = call_payload["content"][0]["text"]
        assert "Status: 200 OK" in call_text
        assert "content-type:" in call_text.lower()

        mcp_rows = _inspect(
            mcp_session,
            vm_name,
            """
            SELECT event_id, server_name, method, tool_name, decision,
                   bytes_sent, bytes_received, arguments AS request_preview,
                   response_preview, trace_id, model_call_id, origin
            FROM tool_calls
            WHERE method = 'tools/call'
              AND origin = 'mcp'
              AND tool_name IN ('http_headers', 'local__http_headers')
            ORDER BY id DESC
            LIMIT 1
            """,
        )
        assert len(mcp_rows) == 1
        mcp_row = mcp_rows[0]
        assert mcp_row["origin"] == "mcp"
        assert mcp_row["model_call_id"] is None
        assert mcp_row["server_name"] == "local"
        assert mcp_row["tool_name"] in {"http_headers", "local__http_headers"}
        assert mcp_row["decision"] == "allowed"
        assert isinstance(mcp_row["event_id"], str) and len(mcp_row["event_id"]) == 12
        assert mcp_row["bytes_sent"] > 0
        assert mcp_row["bytes_received"] > 0
        assert "local__http_headers" in mcp_row["request_preview"]
        assert "Status: 200 OK" in mcp_row["response_preview"]
        assert mcp_row["trace_id"]

        net_rows = _inspect(
            mcp_session,
            vm_name,
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
        assert net_row["domain"] == "127.0.0.1"
        assert net_row["method"] == "GET"
        assert net_row["status_code"] == 200
        assert net_row["decision"] == "allowed"
        assert net_row["bytes_received"] > 0
        assert isinstance(net_row["event_id"], str) and len(net_row["event_id"]) == 12

        security_rows = _inspect(
            mcp_session,
            vm_name,
            f"""
            SELECT event_type, rule_id, rule_action, detection_level,
                   event_json, rule_json
            FROM security_rule_events
            WHERE event_id = '{mcp_row["event_id"]}'
            ORDER BY id
            """,
        )
        assert security_rows
        assert any(row["event_type"] == "mcp.tool_call" for row in security_rows)
        assert any(row["rule_id"] == "profiles.rules.default_mcp" for row in security_rows)
        assert {row["rule_action"] for row in security_rows} <= {"allow", "ask"}
        assert all(row["detection_level"] in {"none", "informational"} for row in security_rows)
        for row in security_rows:
            event = json.loads(row["event_json"])
            rule = json.loads(row["rule_json"])
            assert event["event_type"] == "mcp.tool_call"
            assert event["mcp"]["server_name"] == "local"
            assert event["mcp"]["tool_call_name"] in {"http_headers", "local__http_headers"}
            assert rule["name"]
    finally:
        stop_process(mock_proc)


def test_mcp_call_unknown_tool(shared_vm, mcp_session):
    """Calling a non-existent namespaced tool surfaces an aggregator error.

    This proves the routing chain: MCP stdio -> capsem-service ->
    running-instance MCP aggregator -> error return. Prior to this test,
    nothing in the suite exercised that full path.
    """
    _vm_name, _ = shared_vm
    bogus_name = f"nonexistent__{uuid.uuid4().hex[:8]}"
    resp = mcp_session.call_tool_raw("capsem_mcp_call", {
        "name": bogus_name,
        "arguments": {},
    })
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp, (
        f"expected error calling unknown tool, got: {resp}"
    )


def test_mcp_call_missing_arguments(shared_vm, mcp_session):
    """arguments is optional -- MCP layer defaults it to {} when omitted."""
    _vm_name, _ = shared_vm
    bogus_name = f"nonexistent__{uuid.uuid4().hex[:8]}"
    # No arguments field at all -- should still reach the aggregator and
    # return a tool-not-found error (not a schema validation failure on our side).
    resp = mcp_session.call_tool_raw("capsem_mcp_call", {"name": bogus_name})
    result = resp.get("result", {})
    assert result.get("isError") is True or "error" in resp, (
        f"expected error, got: {resp}"
    )
