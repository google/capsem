"""A workload reaches Capsem's MCP tools at `http://mcp.capsem.internal/mcp`.

A container cannot open vsock, so the in-guest MCP relay is out of its reach.
The session's DNS answers the internal MCP name with 192.0.2.1, the VM's
port-80 rule carries the connection to the host proxy, and the proxy answers
it with that VM's MCP endpoint: the same policy and the same `tool_calls`
ledger as the relay, recorded with `transport = 'http'`. The client here is
the image's own busybox `wget`, run inside the workload by an untargeted exec,
so the request leaves through the workload's network namespace and filter.
"""

import contextlib
import json
import shlex
import sqlite3

import pytest
from helpers.service import exec_output_text, vm_session_db_path
from helpers.session_ledger import open_session_ledger
from helpers.settings_policy import apply_settings_rule

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import created, service

__all__ = ["service"]

pytestmark = pytest.mark.integration

MCP_URL = "http://mcp.capsem.internal/mcp"
# A text only the refused call carries, so the rule cannot match the allowed one.
FORBIDDEN = "kingslanding-forbidden-7f3a"
RULE = "workload_mcp_echo_block"


def in_workload(client, vm_id, command):
    result = client.post(f"/vms/{vm_id}/exec", {"command": command, "timeout_secs": 30}, timeout=40)
    assert result.get("exit_code") == 0, result
    return exec_output_text(result)


def mcp(client, vm_id, message):
    """POST one JSON-RPC message from inside the workload; its JSON answer."""
    body = json.dumps(message, separators=(",", ":"))
    output = in_workload(
        client,
        vm_id,
        f"wget -q -O- --header 'Content-Type: application/json' --post-data {shlex.quote(body)} {MCP_URL}",
    )
    return json.loads(output)


def echo(identifier, text):
    return {
        "jsonrpc": "2.0",
        "id": identifier,
        "method": "tools/call",
        "params": {"name": "local__echo", "arguments": {"text": text}},
    }


def test_a_workload_calls_capsem_mcp_over_http_and_policy_refuses_it(service, tmp_path):
    client = service.client()
    with (
        registry(tmp_path) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "workload-mcp") as vm,
    ):
        vm_id = vm["id"]
        # The name resolves inside the workload, to the reserved address.
        lookup = in_workload(client, vm_id, "nslookup mcp.capsem.internal")
        (tmp_path / "nslookup.txt").write_text(lookup)
        assert "Address: 192.0.2.1" in lookup, lookup

        initialize = mcp(
            client,
            vm_id,
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"clientInfo": {"name": "kingslanding-workload", "version": "1"}},
            },
        )
        assert initialize["id"] == 1, initialize
        assert initialize["result"]["serverInfo"]["name"] == "capsem-mcp-mitm-endpoint", initialize
        assert set(initialize["result"]["capabilities"]) == {"prompts", "resources", "tools"}, (
            initialize
        )

        listed = mcp(client, vm_id, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        names = {tool["name"] for tool in listed["result"]["tools"]}
        assert {
            "local__echo",
            "local__fetch_http",
            "local__grep_http",
            "local__http_headers",
        } <= names, names

        allowed = mcp(client, vm_id, echo(3, "kingslanding-workload-mcp"))
        assert allowed == {
            "jsonrpc": "2.0",
            "id": 3,
            "result": {
                "content": [{"type": "text", "text": "kingslanding-workload-mcp"}],
                "isError": False,
            },
        }, allowed

        # A live rule refuses one call; the workload gets the refusal, not the tool.
        reload = apply_settings_rule(
            service,
            RULE,
            action="block",
            match=f'mcp.tool_call.name.contains("echo") && mcp.request.arguments.contains("{FORBIDDEN}")',
            reason="Kingslanding workload MCP refusal proof.",
        )
        (tmp_path / "rule.json").write_text(json.dumps(reload, indent=2))
        refused = mcp(client, vm_id, echo(4, FORBIDDEN))
        (tmp_path / "refused.json").write_text(json.dumps(refused, indent=2))
        assert refused == {
            "jsonrpc": "2.0",
            "id": 4,
            "error": {
                "code": -32600,
                "message": f"MCP request blocked by security rule: profiles.rules.{RULE}",
            },
        }, refused

        # Nothing else in Capsem's zone resolves: the DNS answers only the
        # MCP name, and asks no upstream resolver for the rest.
        unknown = client.post(
            f"/vms/{vm_id}/exec",
            {"command": "wget -q -O- http://other.capsem.internal/ 2>&1", "timeout_secs": 30},
            timeout=40,
        )
        assert unknown["exit_code"] != 0, unknown
        assert "bad address 'other.capsem.internal'" in exec_output_text(unknown), unknown

        client.post(f"/vms/{vm_id}/stop", {}, timeout=60)
        db_path = vm_session_db_path(service.tmp_dir, client, vm_id)
        with contextlib.closing(open_session_ledger(db_path)) as db:
            db.row_factory = sqlite3.Row
            calls = [
                dict(row)
                for row in db.execute(
                    "SELECT method, tool_name, request_id, transport, origin, server_name, status, decision, "
                    "policy_action, policy_rule, process_name, arguments, response_preview, error_message "
                    "FROM tool_calls WHERE origin = 'mcp' ORDER BY id"
                )
            ]
        (tmp_path / "tool_calls.json").write_text(json.dumps(calls, indent=2))
    # The ledger keeps tool calls; initialize and tools/list are not calls.
    assert [(call["method"], call["transport"]) for call in calls] == [
        ("tools/call", "http")
    ] * 2, calls
    common = {"method": "tools/call", "transport": "http", "origin": "mcp", "server_name": "local"}
    # The client is the image's busybox wget in the workload: the guest finds
    # its socket in the workload's network namespace by its exact address.
    assert [row.pop("process_name") for row in calls] == ["wget"] * 2, calls
    assert calls == [
        {
            **common,
            "tool_name": "local__echo",
            "request_id": "3",
            "status": "responded",
            "decision": "allowed",
            "policy_action": "allow",
            "policy_rule": "profiles.rules.default_mcp",
            "arguments": json.dumps(
                {"arguments": {"text": "kingslanding-workload-mcp"}, "name": "local__echo"},
                separators=(",", ":"),
            ),
            "response_preview": json.dumps(
                {
                    "content": [{"text": "kingslanding-workload-mcp", "type": "text"}],
                    "isError": False,
                },
                separators=(",", ":"),
            ),
            "error_message": None,
        },
        {
            **common,
            "tool_name": "local__echo",
            "request_id": "4",
            "status": "error",
            "decision": "denied",
            "policy_action": "block",
            "policy_rule": f"profiles.rules.{RULE}",
            "arguments": json.dumps(
                {"arguments": {"text": FORBIDDEN}, "name": "local__echo"}, separators=(",", ":")
            ),
            "response_preview": None,
            "error_message": f"MCP request blocked by security rule: profiles.rules.{RULE}",
        },
    ], calls
