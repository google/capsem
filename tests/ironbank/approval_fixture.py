"""Drive a real held operation through authenticated gateway approval."""

from __future__ import annotations

import json
import time
import uuid
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from typing import TypeVar

from helpers.gateway import TcpHttpClient

T = TypeVar("T")


def pending_original_operation(gateway, session_id, original, *, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        status, listing = gateway.call_json("GET", f"/vms/{session_id}/security/asks", timeout=3)
        assert status == 200, (status, listing)
        pending = [row for row in listing["requests"] if row["status"] == "pending"]
        if pending:
            assert len(pending) == listing["pending_count"] == 1, listing
            assert not original.done(), "pending operation already completed"
            return listing
        assert not original.done(), "original operation returned before human resolution"
        assert time.monotonic() < deadline, "original operation never became reviewable"
        time.sleep(0.05)


def assert_original_cancelled(gateway, session_id, listing):
    route = f"/vms/{session_id}/security/asks"
    ask = next(row for row in listing["requests"] if row["status"] == "pending")
    deadline = time.monotonic() + 5
    while True:
        final = gateway.get(route, timeout=2)
        if final["pending_count"] == 0:
            break
        assert time.monotonic() < deadline, "disconnected original retained a live waiter"
        time.sleep(0.05)
    row = next(row for row in final["requests"] if row["ask_id"] == ask["ask_id"])
    assert row["status"] == "cancelled", row
    # This optional projection can briefly be an inactive pending history row.
    # The canonical stopped/drained physical audit below owns the terminal
    # reason assertion; never invent a reason for an absent wire field.
    assert row.get("resolution_reason") in (None, "request_cancelled"), row
    status, _ = gateway.call_json(
        "POST",
        f"{route}/{row['ask_id']}/resolve",
        {
            "generation": listing["generation"],
            "decision": "approve",
        },
    )
    assert status == 409, "cancelled original was approved late"
    return row


def resolve_original_operation(
    operation: Callable[[], T],
    gateway: TcpHttpClient,
    session_id: str,
    decision: str,
    before_resolution: Callable[[], None],
    private_marker: str | None,
) -> tuple[T, dict]:
    """No retries/replay: one operation, one pending identity, one decision."""
    route = f"/vms/{session_id}/security/asks"
    with ThreadPoolExecutor(max_workers=1) as executor:
        original = executor.submit(operation)
        listing = pending_original_operation(gateway, session_id, original)
        pending = [row for row in listing["requests"] if row["status"] == "pending"]
        if private_marker is not None:
            assert private_marker not in json.dumps(listing), (
                "review projection exposed private input"
            )
        before_resolution()
        ask = pending[0]
        resolve = f"{route}/{ask['ask_id']}/resolve"
        body = {"generation": listing["generation"], "decision": decision}
        status, _ = gateway.call_json("POST", resolve, body, use_auth=False)
        assert status == 401, "unauthenticated approval was accepted"
        status, _ = gateway.call_json("POST", resolve, {**body, "generation": str(uuid.uuid4())})
        assert status == 409, "stale generation changed a live waiter"
        wrong_session = f"/vms/{uuid.uuid4()}/security/asks/{ask['ask_id']}/resolve"
        status, _ = gateway.call_json("POST", wrong_session, body)
        assert status == 404, "another session could address this approval"
        before_resolution()
        assert not original.done()
        status, receipt = gateway.call_json("POST", resolve, body)
        assert status == 200, (status, receipt)
        expected = "approved" if decision == "approve" else "denied"
        assert receipt["ask_id"] == ask["ask_id"]
        assert receipt["status"] == expected
        assert receipt["resolution_reason"] == "human_decision"
        status, _ = gateway.call_json("POST", resolve, body)
        assert status == 409, "a duplicate decision was accepted"
        result = original.result(timeout=40)
        status, final = gateway.call_json("GET", route)
        assert status == 200 and final["pending_count"] == 0, final
        final_row = next(row for row in final["requests"] if row["ask_id"] == ask["ask_id"])
        assert final_row["status"] == expected, final
        return result, receipt
