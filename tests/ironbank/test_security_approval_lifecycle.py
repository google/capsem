"""Native approval lifecycle: original operation, authenticated decision, effect."""

from __future__ import annotations

import json
import sqlite3
import textwrap
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing

import pytest
from helpers.service import (
    exec_output_text,
    vm_session_db_path,
)
from helpers.session_ledger import open_session_ledger
from ironbank.approval_fixture import resolve_original_operation
from ironbank.approval_native import native_approval_session, upstream_records

pytestmark = pytest.mark.integration


@pytest.fixture
def approval_vm():
    """Existing local native assets and fake upstream; no vendor account/image pull."""
    with native_approval_session("""
            [corp.rules.ask_native_echo]
            name = "ask_native_echo"
            action = "ask"
            priority = -100
            match = 'http.host == "127.0.0.1" && tcp.port == "3713" && http.path == "/echo"'
        """) as session:
        yield session


def _echo_rows(ready: dict) -> list[dict]:
    return [row for row in upstream_records(ready) if row.get("path") == "/echo"]


@pytest.mark.parametrize("decision", ["approve", "deny"])
def test_original_http_waiter_resolves_once_with_audit(approval_vm, decision: str) -> None:
    service, client, api, session_id, ready = approval_vm
    nonce = uuid.uuid4().hex
    script = textwrap.dedent(f"""
        import json, urllib.request, urllib.error
        body = {json.dumps(nonce)}.encode()
        request = urllib.request.Request({json.dumps(ready["base_url"].rstrip("/") + "/echo")},
            data=body, headers={{'content-type': 'text/plain'}})
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                result = {{'status': response.status, 'body': response.read().decode()}}
        except urllib.error.HTTPError as response:
            result = {{'status': response.code, 'body': response.read().decode()}}
        print('APPROVAL_RESULT=' + json.dumps(result))
    """).strip()
    assert client.upload_file(session_id, "approval-echo.py", script)["success"]

    def no_effect() -> None:
        assert _echo_rows(ready) == [], "held original operation already reached upstream"

    result, receipt = resolve_original_operation(
        lambda: client.post(
            f"/vms/{session_id}/exec",
            {
                "command": "python3 /root/approval-echo.py",
                "timeout_secs": 60,
            },
            timeout=90,
        ),
        api,
        session_id,
        decision,
        no_effect,
        nonce,
    )
    assert result["exit_code"] == 0, result
    outputs = [
        line.removeprefix("APPROVAL_RESULT=")
        for line in exec_output_text(result).splitlines()
        if line.startswith("APPROVAL_RESULT=")
    ]
    assert len(outputs) == 1
    response = json.loads(outputs[0])
    assert response["status"] == (200 if decision == "approve" else 403)
    if decision == "approve":
        assert json.loads(response["body"])["body_size"] == len(nonce)
    assert len(_echo_rows(ready)) == (1 if decision == "approve" else 0)
    # Canonical on-disk ledger, after the trusted resolver's flush barrier.
    path = vm_session_db_path(service.tmp_dir, client, session_id)
    with closing(open_session_ledger(path)) as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            "SELECT status,resolver,reason,event_id FROM security_ask_events "
            "WHERE ask_id = ? ORDER BY id",
            (receipt["ask_id"],),
        ).fetchall()
        assert [row["status"] for row in rows] == ["pending", receipt["status"]]
        assert rows[0]["event_id"] == rows[1]["event_id"] == receipt["event_id"]
        assert rows[1]["resolver"] == "authenticated_client"
        assert rows[1]["reason"] == "human_decision"


def test_disconnected_http_waiter_cancels_without_upstream_effect(approval_vm) -> None:
    _, client, api, session_id, ready = approval_vm
    script = textwrap.dedent(f"""
        import urllib.request
        request = urllib.request.Request({json.dumps(ready["base_url"].rstrip("/") + "/echo")},
            data=b'disconnected-original')
        try:
            urllib.request.urlopen(request, timeout=5)
            raise AssertionError('unresolved request returned a response')
        except TimeoutError:
            print('ORIGINAL_HTTP_CLOSED')
    """).strip()
    assert client.upload_file(session_id, "approval-disconnect.py", script)["success"]
    route = f"/vms/{session_id}/security/asks"
    with ThreadPoolExecutor(max_workers=1) as executor:
        original = executor.submit(
            client.post,
            f"/vms/{session_id}/exec",
            {
                "command": "python3 /root/approval-disconnect.py",
                "timeout_secs": 30,
            },
            timeout=40,
        )
        deadline = time.monotonic() + 3
        while True:
            listing = api.get(route, timeout=2)
            pending = [row for row in listing["requests"] if row["status"] == "pending"]
            if pending:
                break
            assert not original.done() and time.monotonic() < deadline
            time.sleep(0.05)
        assert len(pending) == listing["pending_count"] == 1
        assert _echo_rows(ready) == []
        result = original.result(timeout=15)
        assert result["exit_code"] == 0, result
        assert "ORIGINAL_HTTP_CLOSED" in exec_output_text(result)
    deadline = time.monotonic() + 5
    while True:
        final = api.get(route, timeout=2)
        if final["pending_count"] == 0:
            break
        assert time.monotonic() < deadline, "closed guest HTTP connection retained a live waiter"
        time.sleep(0.05)
    row = next(row for row in final["requests"] if row["ask_id"] == pending[0]["ask_id"])
    assert row["status"] == "cancelled", row
    assert row["resolution_reason"] == "request_cancelled", row
    status, _ = api.call_json(
        "POST",
        f"{route}/{row['ask_id']}/resolve",
        {
            "generation": listing["generation"],
            "decision": "approve",
        },
    )
    assert status == 409
    assert _echo_rows(ready) == []
