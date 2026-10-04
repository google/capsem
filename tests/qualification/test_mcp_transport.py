"""Mandatory for every image: its workload reaches Capsem's MCP over HTTP.

A container cannot open vsock, so a workload's MCP tools live at
`http://mcp.capsem.internal/mcp`, answered by the session's own endpoint under
its policy and ledger. The call is made with the candidate's own `curl` (every
official image has it from base) -- not a client the harness brought -- and
the ledger must name that process as the caller.
"""

import contextlib
import json
import shlex
import sqlite3

import pytest
from helpers.image_session import workload_exec
from helpers.service import vm_session_db_path
from helpers.session_ledger import open_session_ledger

pytestmark = pytest.mark.integration

MCP_URL = "http://mcp.capsem.internal/mcp"


def post(client, vm_id, message):
    body = json.dumps(message, separators=(",", ":"))
    result = workload_exec(
        client,
        vm_id,
        f"curl -sS -H 'Content-Type: application/json' --data {shlex.quote(body)} {MCP_URL}",
    )
    assert result.get("exit_code") == 0, result
    return json.loads(result["stdout_text"])


def test_the_workload_calls_capsem_mcp_and_the_ledger_names_its_process(
    service, candidate, session
):
    client = service.client()
    hello = post(client, session, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
    assert hello["result"]["serverInfo"]["name"] == "capsem-mcp-mitm-endpoint", hello
    tools = post(client, session, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
    assert "local__echo" in {tool["name"] for tool in tools["result"]["tools"]}, tools
    echoed = post(
        client,
        session,
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "local__echo", "arguments": {"text": f"qualify-{candidate.name}"}},
        },
    )
    assert echoed["result"]["content"] == [{"type": "text", "text": f"qualify-{candidate.name}"}]

    client.post(f"/vms/{session}/stop", {}, timeout=60)
    with contextlib.closing(open_session_ledger(vm_session_db_path(service.tmp_dir, client, session))) as db:
        db.row_factory = sqlite3.Row
        calls = [
            dict(row)
            for row in db.execute(
                "SELECT tool_name, transport, decision, process_name FROM tool_calls "
                "WHERE origin = 'mcp' ORDER BY id"
            )
        ]
    assert calls == [
        {"tool_name": "local__echo", "transport": "http", "decision": "allowed", "process_name": "curl"}
    ], calls
