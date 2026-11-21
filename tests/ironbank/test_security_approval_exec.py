"""Native public exec waits before any guest command effect."""

import json
import shlex
import time
import uuid
from concurrent.futures import ThreadPoolExecutor

import pytest
from helpers.service import exec_output_text
from ironbank.approval_fixture import (
    assert_original_cancelled,
    pending_original_operation,
    resolve_original_operation,
)
from ironbank.approval_native import (
    assert_approval_audit,
    native_approval_session,
)
from websockets.sync.client import connect

pytestmark = pytest.mark.integration
EFFECT = "/tmp/approval-exec-effects"
POLICY = """
[corp.rules.ask_native_exec]
name = "ask_native_exec"
action = "ask"
priority = -100
match = 'process.command.contains("ASK_EXEC_ORIGINAL")'
"""


def command(nonce):
    return f"printf '%s\\n' {shlex.quote(nonce)} >> {EFFECT}; : ASK_EXEC_ORIGINAL"


def effects(client, vm_id):
    result = client.post(
        f"/vms/{vm_id}/exec",
        {
            "command": f"if [ -f {EFFECT} ]; then cat {EFFECT}; fi",
            "target": "vm",
            "timeout_secs": 10,
        },
        timeout=15,
    )
    assert result["exit_code"] == 0, result
    return exec_output_text(result).splitlines()


@pytest.mark.parametrize("decision", ["approve", "deny"])
def test_exec_original_command_resolves_once(decision):
    resolve_exec(decision, hold_seconds=0)


@pytest.mark.parametrize("decision", ["approve", "deny"])
def test_exec_approval_events_arrive_without_status_polling(decision):
    with native_approval_session(POLICY) as (service, client, api, vm_id, _):
        endpoint = api.base_url.replace("http://", "ws://", 1) + "/events"
        route = f"/vms/{vm_id}/security/asks"
        event = {"type": "approval-pending", "payload": {"id": vm_id}}
        nonce = uuid.uuid4().hex
        with (
            connect(
                endpoint,
                additional_headers={"Authorization": f"Bearer {api.token}"},
                open_timeout=5,
                close_timeout=2,
            ) as socket,
            ThreadPoolExecutor(max_workers=1) as executor,
        ):
            original = executor.submit(
                api.post,
                f"/vms/{vm_id}/exec",
                {"command": command(nonce), "target": "vm", "timeout_secs": 15},
                timeout=30,
            )
            assert json.loads(socket.recv(timeout=5)) == event
            listing = api.get(route)
            assert listing["pending_count"] == 1
            ask = next(row for row in listing["requests"] if row["status"] == "pending")
            assert effects(client, vm_id) == []
            receipt = api.post(
                f"{route}/{ask['ask_id']}/resolve",
                {"generation": listing["generation"], "decision": decision},
            )
            assert json.loads(socket.recv(timeout=5)) == event
            assert api.get(route)["pending_count"] == 0
            assert (original.result(timeout=10)["exit_code"] == 0) == (
                decision == "approve"
            )
        assert effects(client, vm_id) == ([nonce] if decision == "approve" else [])
        assert_approval_audit(service, client, vm_id, receipt, table="exec_events")


@pytest.mark.parametrize("decision", ["approve", "deny"])
def test_exec_original_survives_human_review_beyond_execution_budget(decision):
    resolve_exec(decision, hold_seconds=31)


def resolve_exec(decision, *, hold_seconds):
    with native_approval_session(POLICY) as (service, client, api, vm_id, _):
        nonce = uuid.uuid4().hex
        review_started = False

        def no_effect():
            nonlocal review_started
            if not review_started:
                review_started = True
                time.sleep(hold_seconds)
            assert effects(client, vm_id) == [], "held command already executed"

        result, receipt = resolve_original_operation(
            lambda: client.post(
                f"/vms/{vm_id}/exec",
                {
                    "command": command(nonce),
                    "target": "vm",
                    "timeout_secs": 30,
                },
                timeout=90 if hold_seconds else 40,
            ),
            api,
            vm_id,
            decision,
            no_effect,
            nonce,
        )
        assert (result["exit_code"] == 0) == (decision == "approve"), result
        assert effects(client, vm_id) == ([nonce] if decision == "approve" else [])
        assert_approval_audit(service, client, vm_id, receipt, table="exec_events")


def test_exec_caller_disconnect_cancels_original_command():
    with native_approval_session(POLICY) as (service, client, api, vm_id, _):
        nonce = uuid.uuid4().hex
        with ThreadPoolExecutor(max_workers=1) as executor:
            original = executor.submit(
                client.post,
                f"/vms/{vm_id}/exec",
                {
                    "command": command(nonce),
                    "target": "vm",
                    "timeout_secs": 30,
                },
                timeout=3,
            )
            listing = pending_original_operation(api, vm_id, original, timeout=2)
            assert effects(client, vm_id) == []
            with pytest.raises(ConnectionError):
                original.result(timeout=10)
        receipt = assert_original_cancelled(api, vm_id, listing)
        assert effects(client, vm_id) == []
        assert_approval_audit(service, client, vm_id, receipt, cancelled=True, table="exec_events")
