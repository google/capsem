"""The shipped agent invokes its configured MCP connection through Capsem."""

from __future__ import annotations

import contextlib
import json
import re
import shlex
import sqlite3
import uuid
from datetime import datetime

import pytest
from helpers.body_archive import SessionArchive, security_payload
from helpers.image_session import WORKSPACE, session_of, workload_exec
from helpers.service import vm_session_db_path
from helpers.session_ledger import open_session_ledger

from tests.qualification.test_agent import model_service

__all__ = ["model_service"]
pytestmark = pytest.mark.integration


@pytest.mark.capability("agent")
def test_the_agent_invokes_its_configured_capsem_mcp_echo(
    model_service, candidate, tmp_path
):
    expect = candidate.expect
    assert expect.get("turn"), f"{candidate.name} declares agent but no [expect] turn"
    token = uuid.uuid4().hex
    prompt = (
        f"CAPSEM_MCP_PROOF={token}. Call the Capsem MCP local echo tool with text {token}. "
        "Return only the actual text returned by that tool."
    )
    command = shlex.join([*expect["turn"], prompt])
    credential = f"{expect['credential_env']}=capsem-qualification-test-key"
    client = model_service.client()
    with session_of(
        model_service,
        tmp_path / "registry",
        "agent-mcp",
        layout=candidate.layout,
        image_name=candidate.name,
        env={expect["credential_env"]: "capsem-qualification-test-key"},
    ) as vm_id:
        try:
            turn = workload_exec(
                client,
                vm_id,
                f"cd {WORKSPACE} && env {credential} {command}",
                timeout=240,
            )
        finally:
            logs = client.get(f"/vms/{vm_id}/logs")
            (model_service.home_dir / "agent-runtime-logs.json").write_text(
                json.dumps(logs, indent=2)
            )
            transcript = tmp_path / "upstream-transcript.jsonl"
            if transcript.exists():
                (model_service.home_dir / "agent-upstream.jsonl").write_bytes(
                    transcript.read_bytes()
                )
        assert turn.get("exit_code") == 0, turn
        assert turn["stdout_text"].strip() == token, turn
        client.post(f"/vms/{vm_id}/stop", {}, timeout=60)
        assert_agent_ledger(
            vm_session_db_path(model_service.tmp_dir, client, vm_id), expect, token
        )


def assert_agent_ledger(db_path, expect, token):
    with contextlib.closing(open_session_ledger(db_path)) as db:
        db.row_factory = sqlite3.Row
        calls = [
            dict(row)
            for row in db.execute(
                "SELECT * FROM tool_calls WHERE origin = 'mcp' ORDER BY id"
            )
        ]
        assert len(calls) == 1, calls
        call = calls[0]
        assert set(call) == {
            "id",
            "event_id",
            "timestamp",
            "model_call_id",
            "provider",
            "status",
            "call_index",
            "call_id",
            "tool_name",
            "arguments",
            "response_preview",
            "origin",
            "transport",
            "server_name",
            "method",
            "request_id",
            "decision",
            "duration_ms",
            "error_message",
            "process_name",
            "bytes_sent",
            "bytes_received",
            "policy_mode",
            "policy_action",
            "policy_rule",
            "policy_reason",
            "trace_id",
            "turn_id",
            "credential_ref",
        }, call
        assert (
            call["id"] > 0
            and datetime.fromisoformat(call["timestamp"]).tzinfo is not None
        ), call
        assert (
            call["model_call_id"] is None
            and call["provider"] == ""
            and call["credential_ref"] is None
        ), call
        assert call["call_index"] == 0 and call["origin"] == "mcp", call
        assert call["call_id"] == call["request_id"] and call["call_id"], call
        assert call["turn_id"] == call["trace_id"], call
        assert (
            call["policy_mode"] == "security_event"
            and isinstance(call["policy_reason"], str)
            and call["policy_reason"]
        ), call
        assert call["tool_name"] == "local__echo" and call["transport"] == "http", call
        assert call["decision"] == "allowed" and call["status"] == "responded", call
        assert call["process_name"] == expect["turn"][0], call
        assert call["error_message"] is None, call
        assert call["method"] == "tools/call" and call["server_name"] == "local", call
        assert re.fullmatch("[0-9a-f]{12}", call["event_id"]), call
        assert call["trace_id"], call
        assert (
            call["duration_ms"] >= 0
            and call["bytes_sent"] > 0
            and call["bytes_received"] > 0
        ), call
        request = json.loads(call["arguments"])
        response = json.loads(call["response_preview"])
        assert set(request) <= {"name", "arguments", "_meta"}, request
        assert request["name"] == "local__echo" and request["arguments"] == {
            "text": token
        }, request
        if expect["provider"] == "anthropic":
            assert request["_meta"] == {
                "claudecode/toolUseId": "toolu_capsem_mcp_echo",
                "progressToken": int(call["request_id"]),
            }, request
        assert response == {
            "content": [{"type": "text", "text": token}],
            "isError": False,
        }, response
        assert (
            call["policy_action"] == "allow"
            and call["policy_rule"] == "profiles.rules.default_mcp"
        ), call
        rules = [
            dict(row)
            for row in db.execute(
                "SELECT event_type, rule_id, rule_action, trace_id FROM security_rule_events WHERE event_id = ?",
                (call["event_id"],),
            )
        ]
        assert rules, call
        assert all(
            rule["trace_id"] == call["trace_id"] and rule["rule_action"] == "allow"
            for rule in rules
        ), rules
        event = security_payload(db, call["event_id"])
        assert (
            event["event_type"] == "mcp.tool_call"
            and event["decision"]["effective"] == "allow"
        ), event
        assert event["process"]["name"] == expect["turn"][0], event
        assert (
            event["mcp"]["tool_call_name"] == "local__echo"
            and event["mcp"]["server_name"] == "local"
        ), event
        assert event["mcp"]["request"]["id"] == call["request_id"], event
        assert event["mcp"]["request"]["arguments"] == {"text": token}, event
        assert event["mcp"]["response"]["content"] == response["content"], event
        models = [
            dict(row)
            for row in db.execute(
                "SELECT event_id, provider, path, method, model, process_name, tools_count, status_code, text_content, stop_reason FROM model_calls WHERE method = 'POST'"
            )
        ]
        with SessionArchive(db_path) as archive:
            request_body = archive.read(call["event_id"], "tool_calls", "request")
            response_body = archive.read(call["event_id"], "tool_calls", "response")
            assert request_body is not None and response_body is not None, call
            assert json.loads(request_body) == request
            assert json.loads(response_body) == response
            turns = []
            for model in models:
                body = archive.read(model["event_id"], "model_calls", "request")
                assert body is not None, model
                if contains_proof_prompt(json.loads(body), token):
                    turns.append(model)
            for model in turns:
                if expect["provider"] == "openai" and model["text_content"] == token:
                    body = archive.read(model["event_id"], "model_calls", "response")
                    assert body is not None, model
                    events = [
                        json.loads(line.removeprefix("data: "))
                        for line in body.decode().splitlines()
                        if line.startswith("data: ")
                    ]
                    assert events[-1]["type"] == "response.completed", events
                    final_response = events[-1]["response"]
                    assert (
                        final_response["status"] == "completed"
                        and final_response["error"] is None
                    ), final_response
                    assert final_response["output"][0]["content"][0]["text"] == token, (
                        final_response
                    )
        assert turns, models
        assert all(
            model["provider"] == expect["provider"]
            and model["path"] == expect["path"]
            and model["method"] == "POST"
            and model["model"]
            and model["process_name"] == expect["turn"][0]
            and model["status_code"] == 200
            for model in turns
        ), turns
        assert any(
            model["text_content"] == token and model["stop_reason"] == "end_turn"
            for model in turns
        ), turns


def contains_proof_prompt(payload, token):
    for message in payload.get("messages", payload.get("input", [])):
        if message.get("role") != "user":
            continue
        content = message.get("content", [])
        if isinstance(content, str):
            content = [{"text": content}]
        if any(
            block.get("text", "").lstrip().startswith(f"CAPSEM_MCP_PROOF={token}.")
            for block in content
        ):
            return True
    return False
