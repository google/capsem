"""Capability `agent`: the shipped agent completes a turn through Capsem.

The agent runs as it ships -- its own binary, its own configuration, nothing
written for the test -- and dials its real provider host. Capsem's corp
upstream overrides deliver that connection to the hermetic mock model server
(helpers.hermetic_models), which answers with a tool call that writes a token
into the workspace. Qualification needs the observable outcome: the turn
completes, the file holds the token, and the session ledger records the
model call against the provider the image declares.
"""

import contextlib
import os
import shlex
import sqlite3
import uuid

import pytest
from helpers.hermetic_models import hermetic_models
from helpers.image_session import WORKSPACE, session_of, workload_exec
from helpers.service import ServiceInstance, vm_session_db_path
from helpers.session_ledger import open_session_ledger

pytestmark = pytest.mark.integration


@pytest.fixture
def model_service(tmp_path):
    """A service whose corp config routes every model provider to the mock."""
    with hermetic_models(tmp_path) as (_, corp):
        previous = os.environ.get("CAPSEM_CORP_CONFIG")
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp)
        service = ServiceInstance()
        try:
            service.start()
            yield service
        finally:
            service.stop()
            if previous is None:
                os.environ.pop("CAPSEM_CORP_CONFIG", None)
            else:
                os.environ["CAPSEM_CORP_CONFIG"] = previous


@pytest.mark.capability("agent")
def test_the_agent_completes_a_turn_and_its_tool_call_lands(
    model_service, candidate, tmp_path
):
    expect = candidate.expect
    assert expect.get("turn"), f"{candidate.name} declares agent but no [expect] turn"
    token, target = uuid.uuid4().hex, f"{WORKSPACE}/agent-turn.txt"
    prompt = f"Write uuid4 hex value {token} to {target}."
    command = shlex.join([*expect["turn"], prompt])
    credential = f"{expect['credential_env']}=capsem-qualification-test-key"
    client = model_service.client()
    with session_of(
        model_service,
        tmp_path / "registry",
        "agent",
        layout=candidate.layout,
        image_name=candidate.name,
    ) as vm_id:
        turn = workload_exec(
            client, vm_id, f"cd {WORKSPACE} && env {credential} {command}", timeout=240
        )
        assert turn.get("exit_code") == 0, turn
        written = workload_exec(client, vm_id, f"cat {target}")
        assert written["stdout_text"].strip() == token, (turn, written)
        client.post(f"/vms/{vm_id}/stop", {}, timeout=60)
        db_path = vm_session_db_path(model_service.tmp_dir, client, vm_id)
        with contextlib.closing(open_session_ledger(db_path)) as db:
            db.row_factory = sqlite3.Row
            calls = [
                dict(row)
                for row in db.execute(
                    "SELECT id, provider, path, model, process_name, method, status_code, text_content, stop_reason, trace_id FROM model_calls ORDER BY id"
                )
            ]
            if expect.get("response_transport") == "websocket":
                assert len(calls) == 2, calls
                assert all(
                    row["method"] == "GET" and row["status_code"] == 101
                    for row in calls
                ), calls
                assert calls[0]["stop_reason"] == "tool_use", calls
                assert (
                    calls[-1]["text_content"] == token
                    and calls[-1]["stop_reason"] == "end_turn"
                ), calls
                assert calls[0]["trace_id"] == calls[-1]["trace_id"], calls
                proposed = [
                    dict(row)
                    for row in db.execute(
                        "SELECT call_id, tool_name, arguments, transport FROM tool_calls WHERE model_call_id=?",
                        (calls[0]["id"],),
                    )
                ]
                assert (
                    len(proposed) == 1 and proposed[0]["tool_name"] == "exec_command"
                ), proposed
                assert proposed[0]["transport"] == "websocket", proposed
                assert (
                    token in proposed[0]["arguments"]
                    and target in proposed[0]["arguments"]
                ), proposed
                result_ids = [
                    row[0]
                    for row in db.execute(
                        "SELECT call_id FROM tool_responses WHERE model_call_id=?",
                        (calls[-1]["id"],),
                    )
                ]
                assert result_ids == [proposed[0]["call_id"]], result_ids
    assert any(
        call["provider"] == expect["provider"]
        and call["path"] == expect["path"]
        and call["process_name"] == expect["turn"][0]
        and call["model"]
        for call in calls
    ), calls
