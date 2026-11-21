"""Real HTTP waiter expiry, separation and retirement through the existing VM."""

from __future__ import annotations

import json
import time
import uuid
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from contextlib import closing

import pytest
from helpers.service import exec_output_text, vm_session_db_path
from helpers.session_ledger import open_session_ledger
from websockets.sync.client import connect

from tests.ironbank.approval_fixture import pending_original_operation
from tests.ironbank.approval_native import (
    assert_approval_audits,
    native_approval_session,
)
from tests.ironbank.test_security_approval_lifecycle import (
    _echo_rows,
)
from tests.ironbank.test_security_approval_lifecycle import (
    approval_vm as approval_vm,  # noqa: PLC0414 - re-export the pytest fixture
)

pytestmark = pytest.mark.integration


def original_http(client, api, session_id, ready, marker, *, request_timeout=30):
    # Only the expiry case permits the broker's real published deadline to
    # elapse. The other cases retain an ordinary finite caller budget.
    source = f"""
import json, urllib.request, urllib.error
request = urllib.request.Request({json.dumps(ready["base_url"].rstrip("/") + "/echo")},
    data={marker!r}.encode(), headers={{'content-type': 'text/plain'}})
try:
    with urllib.request.urlopen(request, timeout={request_timeout}) as response:
        result = {{'status': response.status, 'body': response.read().decode()}}
except urllib.error.HTTPError as response:
    result = {{'status': response.code, 'body': response.read().decode()}}
print('APPROVAL_RESULT=' + json.dumps(result))
"""
    uploaded = client.upload_file(session_id, f"approval-{marker}.py", source)
    assert uploaded["success"], uploaded

    def operation():
        return api.call_json(
            "POST",
            f"/vms/{session_id}/exec",
            {
                "command": f"python3 {uploaded['vm_path']}",
                "target": "vm",
                "timeout_secs": request_timeout + 10,
            },
            timeout=request_timeout + 30,
        )

    return operation


def response(original):
    status, result = original.result(timeout=150)
    assert status == 200, (status, result)
    assert result["exit_code"] == 0, result
    lines = [
        line.removeprefix("APPROVAL_RESULT=")
        for line in exec_output_text(result).splitlines()
        if line.startswith("APPROVAL_RESULT=")
    ]
    assert len(lines) == 1, result
    return json.loads(lines[0])


def listing(api, session_id):
    status, result = api.call_json("GET", f"/vms/{session_id}/security/asks")
    assert status == 200, (status, result)
    return result


def resolve(api, session_id, generation, receipt, decision, expected):
    status, result = api.call_json(
        "POST",
        f"/vms/{session_id}/security/asks/{receipt['ask_id']}/resolve",
        {"generation": generation, "decision": decision},
    )
    assert status == expected, (status, result)
    return result


def test_committed_request_denial_arrives_without_status_polling():
    with native_approval_session("""
        [corp.rules.deny_native_mock_echo]
        name = "deny_native_mock_echo"
        action = "block"
        priority = -100
        match = 'http.host == "127.0.0.1" && tcp.port == "3713" && http.path == "/echo"'
    """) as (service, client, api, session_id, ready):
        endpoint = api.base_url.replace("http://", "ws://", 1) + "/events"
        headers = {"Authorization": f"Bearer {api.token}"}
        operation = original_http(client, api, session_id, ready, uuid.uuid4().hex)
        with (
            connect(endpoint, additional_headers=headers, open_timeout=5, close_timeout=2) as socket,
            ThreadPoolExecutor(max_workers=1) as executor,
        ):
            original = executor.submit(operation)
            assert json.loads(socket.recv(timeout=10)) == {
                "type": "request-denied",
                "payload": {"id": session_id},
            }
            ledger = vm_session_db_path(service.tmp_dir, client, session_id)
            with closing(open_session_ledger(ledger)) as db:
                count = db.execute(
                    "SELECT count(*) FROM net_events WHERE path = ? AND decision = ?",
                    ("/echo", "denied"),
                ).fetchone()[0]
                assert count == 1
            assert response(original)["status"] == 403
            assert _echo_rows(ready) == []
        with (
            connect(endpoint, additional_headers=headers, open_timeout=5, close_timeout=2) as socket,
            pytest.raises(TimeoutError),
        ):
            socket.recv(timeout=0.75)


def test_original_http_expires_without_dispatch_or_late_authorization(approval_vm):
    service, client, api, session_id, ready = approval_vm
    marker = uuid.uuid4().hex
    with ThreadPoolExecutor(max_workers=1) as executor:
        original = executor.submit(
            original_http(client, api, session_id, ready, marker, request_timeout=180)
        )
        pending = pending_original_operation(api, session_id, original)
        ask = next(row for row in pending["requests"] if row["status"] == "pending")
        assert ask["expires_at_unix_ms"] > time.time_ns() // 1_000_000
        assert marker not in json.dumps(pending)
        assert _echo_rows(ready) == []
        result = response(original)
        assert time.time_ns() // 1_000_000 >= ask["expires_at_unix_ms"]
        assert result["status"] == 403, result
        assert marker not in result["body"]
    final = listing(api, session_id)
    assert final["pending_count"] == 0, final
    receipt = next(row for row in final["requests"] if row["ask_id"] == ask["ask_id"])
    assert receipt["status"] == "expired", receipt
    resolve(api, session_id, pending["generation"], receipt, "approve", 409)
    assert _echo_rows(ready) == []
    assert_approval_audits(service, client, session_id, [(receipt, False)], table="net_events")


def test_concurrent_http_waiters_resolve_independently_without_replay(approval_vm):
    service, client, api, session_id, ready = approval_vm
    markers = [uuid.uuid4().hex, uuid.uuid4().hex + "-second"]
    operations = [original_http(client, api, session_id, ready, marker) for marker in markers]
    with ThreadPoolExecutor(max_workers=2) as executor:
        originals = [executor.submit(operation) for operation in operations]
        deadline = time.monotonic() + 10
        while True:
            pending = listing(api, session_id)
            assert all(not original.done() for original in originals)
            if pending["pending_count"] == 2:
                break
            assert pending["pending_count"] in (0, 1), pending
            assert time.monotonic() < deadline, pending
            time.sleep(0.05)
        asks = [row for row in pending["requests"] if row["status"] == "pending"]
        assert len(asks) == len({row["ask_id"] for row in asks}) == 2
        assert all(marker not in json.dumps(pending) for marker in markers)
        assert _echo_rows(ready) == []
        approved = resolve(api, session_id, pending["generation"], asks[0], "approve", 200)
        completed, unfinished = wait(originals, timeout=10, return_when=FIRST_COMPLETED)
        assert len(completed) == len(unfinished) == 1
        first = next(iter(completed))
        result = response(first)
        assert result["status"] == 200, result
        assert json.loads(result["body"])["body_size"] == len(markers[originals.index(first)])
        remaining = listing(api, session_id)
        assert remaining["pending_count"] == 1, remaining
        assert len(_echo_rows(ready)) == 1
        denied = resolve(api, session_id, pending["generation"], asks[1], "deny", 200)
        result = response(next(iter(unfinished)))
        assert result["status"] == 403, result
        assert all(marker not in result["body"] for marker in markers)
    assert listing(api, session_id)["pending_count"] == 0
    for receipt in (approved, denied):
        resolve(api, session_id, pending["generation"], receipt, "approve", 409)
    assert len(_echo_rows(ready)) == 1
    assert_approval_audits(
        service, client, session_id, [(approved, False), (denied, False)], table="net_events"
    )


def test_session_retirement_cancels_original_http_before_any_upstream_effect(approval_vm):
    service, client, api, session_id, ready = approval_vm
    marker = uuid.uuid4().hex
    with ThreadPoolExecutor(max_workers=1) as executor:
        original = executor.submit(original_http(client, api, session_id, ready, marker))
        pending = pending_original_operation(api, session_id, original)
        ask = next(row for row in pending["requests"] if row["status"] == "pending")
        assert _echo_rows(ready) == []
        assert_approval_audits(service, client, session_id, [(ask, True)], table="net_events")
        status, result = original.result(timeout=15)
        assert status == 500, (status, result)
        assert marker not in json.dumps(result)
    assert _echo_rows(ready) == []
    # Retirement removes live authority. Its canonical stopped/drained ledger
    # must still retain the original cancellation rather than approving later.
    status, result = api.call_json(
        "POST",
        f"/vms/{session_id}/security/asks/{ask['ask_id']}/resolve",
        {"generation": pending["generation"], "decision": "approve"},
    )
    assert status == 404, (status, result)
