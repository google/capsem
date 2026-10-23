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


def resolve_original_operation(
    operation: Callable[[], T],
    gateway: TcpHttpClient,
    session_id: str,
    decision: str,
    before_resolution: Callable[[], None],
    private_marker: str,
) -> tuple[T, dict]:
    """No retries/replay: one operation, one pending identity, one decision."""
    route = f"/vms/{session_id}/security/asks"
    with ThreadPoolExecutor(max_workers=1) as executor:
        original = executor.submit(operation)
        deadline = time.monotonic() + 10
        while True:
            status, listing = gateway.call_json("GET", route, timeout=3)
            assert status == 200, (status, listing)
            pending = [row for row in listing["requests"] if row["status"] == "pending"]
            if pending:
                break
            assert not original.done(), "original operation returned before human resolution"
            assert time.monotonic() < deadline, "original operation never became reviewable"
            time.sleep(0.05)
        assert len(pending) == listing["pending_count"] == 1, listing
        assert private_marker not in json.dumps(listing), "review projection exposed private input"
        assert not original.done(), "pending operation already completed"
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
