"""Native direct, internal HTTP and observed MCP original-call acceptance."""

import json
import textwrap
import uuid
from concurrent.futures import ThreadPoolExecutor

import pytest
from helpers.service import exec_output_text

from tests.ironbank.approval_fixture import (
    assert_original_cancelled,
    pending_original_operation,
    resolve_original_operation,
)
from tests.ironbank.approval_native import (
    assert_approval_audit,
    native_approval_session,
    upstream_records,
)

pytestmark = pytest.mark.integration
SURFACES = ["direct", "internal-http", "observed-http"]
DIRECT = "/mcp/servers/local/tools/local__http_headers/call"
POLICY = """
[corp.rules.ask_native_mcp]
name = "ask_native_mcp"
action = "ask"
priority = -100
match = 'mcp.method == "tools/call" && mcp.response.valid == "false"'
"""


def arguments(ready, nonce):
    return {"url": ready["base_url"].rstrip("/") + "/html/about?approval=" + nonce, "method": "GET"}


def effects(ready, surface, nonce):
    rows = upstream_records(ready)
    if surface == "observed-http":
        return [
            row
            for row in rows
            if row.get("path") == "/mcp" and nonce in str(row.get("request_body", ""))
        ]
    return [
        row
        for row in rows
        if row.get("path") == "/html/about" and nonce in (row.get("query") or "")
    ]


def guest_script(ready, surface, nonce, *, cancel=False):
    observed = surface == "observed-http"
    url = ready["base_url"].rstrip("/") + "/mcp" if observed else "http://mcp.capsem.internal/mcp"
    message = {
        "jsonrpc": "2.0",
        "id": 731,
        "method": "tools/call",
        "params": {
            "name": "fixture_lookup" if observed else "local__http_headers",
            "arguments": {"query": nonce} if observed else arguments(ready, nonce),
        },
    }
    return textwrap.dedent(f"""
        import json, urllib.request, urllib.error
        request = urllib.request.Request({url!r}, data={json.dumps(message)!r}.encode(),
            headers={{'Content-Type':'application/json'}})
        try:
            with urllib.request.urlopen(request, timeout={5 if cancel else 30}) as response:
                result = {{'status':response.status,'body':response.read().decode()}}
        except urllib.error.HTTPError as response:
            result = {{'status':response.code,'body':response.read().decode()}}
        except TimeoutError:
            assert {cancel!r}, 'unexpected MCP timeout'
            result = {{'closed':True}}
        print('MCP_APPROVAL_RESULT=' + json.dumps(result))
    """).strip()


def guest_result(result):
    assert result["exit_code"] == 0, result
    lines = [
        line.split("=", 1)[1]
        for line in exec_output_text(result).splitlines()
        if line.startswith("MCP_APPROVAL_RESULT=")
    ]
    assert len(lines) == 1
    return json.loads(lines[0])


def operation(client, api, vm_id, surface, ready, nonce, uploaded=None, *, timeout=40):
    if surface == "direct":
        return api.call_json("POST", DIRECT, arguments(ready, nonce), timeout=timeout)
    assert uploaded is not None
    result = client.post(
        f"/vms/{vm_id}/exec",
        {
            "command": f"python3 {uploaded['vm_path']}",
            "target": "vm",
            "timeout_secs": 40,
        },
        timeout=50,
    )
    decoded = guest_result(result)
    return decoded["status"], decoded["body"]


@pytest.mark.parametrize("surface", SURFACES)
@pytest.mark.parametrize("decision", ["approve", "deny"])
def test_mcp_original_call_resolves_once(surface, decision):
    with native_approval_session(POLICY) as (service, client, api, vm_id, ready):
        nonce = uuid.uuid4().hex
        uploaded = (
            None
            if surface == "direct"
            else client.upload_file(vm_id, "approval-mcp.py", guest_script(ready, surface, nonce))
        )
        if uploaded is not None:
            assert uploaded["success"]

        def no_effect():
            assert effects(ready, surface, nonce) == [], "held original tool reached upstream"

        result, receipt = resolve_original_operation(
            lambda: operation(client, api, vm_id, surface, ready, nonce, uploaded),
            api,
            vm_id,
            decision,
            no_effect,
            nonce,
        )
        status, payload = result
        expected = (
            200
            if decision == "approve" or surface == "internal-http"
            else (502 if surface == "direct" else 403)
        )
        assert status == expected, result
        if decision == "approve":
            envelope = payload if isinstance(payload, dict) else json.loads(payload)
            assert "error" not in envelope and envelope["result"]["isError"] is False, envelope
            text = envelope["result"]["content"][0]["text"]
            assert ("fixture_lookup" if surface == "observed-http" else "Status: 200 OK") in text
        elif surface == "internal-http":
            assert "error" in json.loads(payload), payload
        assert len(effects(ready, surface, nonce)) == (1 if decision == "approve" else 0)
        assert_approval_audit(service, client, vm_id, receipt, table="tool_calls")


@pytest.mark.parametrize("surface", SURFACES)
def test_mcp_caller_disconnect_cancels_original_call(surface):
    with native_approval_session(POLICY) as (service, client, api, vm_id, ready):
        nonce = uuid.uuid4().hex
        uploaded = (
            None
            if surface == "direct"
            else client.upload_file(
                vm_id, "approval-mcp-close.py", guest_script(ready, surface, nonce, cancel=True)
            )
        )
        with ThreadPoolExecutor(max_workers=1) as executor:
            if surface == "direct":
                original = executor.submit(
                    operation, client, api, vm_id, surface, ready, nonce, timeout=3
                )
            else:
                assert uploaded is not None
                assert uploaded["success"]
                original = executor.submit(
                    client.post,
                    f"/vms/{vm_id}/exec",
                    {
                        "command": f"python3 {uploaded['vm_path']}",
                        "target": "vm",
                        "timeout_secs": 30,
                    },
                    timeout=40,
                )
            listing = pending_original_operation(api, vm_id, original, timeout=2)
            assert effects(ready, surface, nonce) == []
            if surface == "direct":
                with pytest.raises(ConnectionError):
                    original.result(timeout=10)
            else:
                assert guest_result(original.result(timeout=15)) == {"closed": True}
        receipt = assert_original_cancelled(api, vm_id, listing)
        assert effects(ready, surface, nonce) == []
        assert_approval_audit(service, client, vm_id, receipt, cancelled=True)
