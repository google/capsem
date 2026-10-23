"""Native public VirtioFS import/export preserves the original held bytes."""

import json
import shlex
import uuid
from concurrent.futures import ThreadPoolExecutor

import pytest
from helpers.service import exec_output_text
from helpers.settings_policy import apply_settings_rule
from ironbank.approval_fixture import (
    assert_original_cancelled,
    pending_original_operation,
    resolve_original_operation,
)
from ironbank.approval_native import (
    assert_approval_audit,
    native_approval_session,
)

pytestmark = pytest.mark.integration
NAME = "approval-boundary.txt"
SEED = b"original-guest-file"


def prepare(service, client, vm_id, action):
    seed = client.upload_file(vm_id, NAME, SEED)
    assert seed["success"], seed
    apply_settings_rule(
        service,
        "ask_native_file",
        action="ask",
        priority=0,
        match=f'file.{action}.path.endsWith("{NAME}")',
    )
    return seed["vm_path"]


def guest_bytes(client, vm_id, path):
    result = client.post(
        f"/vms/{vm_id}/exec",
        {
            "command": f"cat {shlex.quote(path)}",
            "target": "vm",
            "timeout_secs": 10,
        },
        timeout=15,
    )
    assert result["exit_code"] == 0, result
    return exec_output_text(result).encode()


def original_file_operation(client, vm_id, action, data, *, timeout=15):
    return client.call(
        "POST" if action == "import" else "GET",
        f"/vms/{vm_id}/files/content?path={NAME}",
        body=data if action == "import" else None,
        headers={"Content-Type": "application/octet-stream"},
        timeout=timeout,
    )


@pytest.mark.parametrize("action", ["import", "export"])
@pytest.mark.parametrize("decision", ["approve", "deny"])
def test_file_original_boundary_resolves_once(action, decision):
    with native_approval_session() as (service, client, api, vm_id, _):
        path = prepare(service, client, vm_id, action)
        data = uuid.uuid4().hex.encode()

        def no_effect():
            assert guest_bytes(client, vm_id, path) == SEED

        result, receipt = resolve_original_operation(
            lambda: original_file_operation(client, vm_id, action, data),
            api,
            vm_id,
            decision,
            no_effect,
            data.decode() if action == "import" else SEED.decode(),
        )
        status, _, body = result
        assert status == (200 if decision == "approve" else 500), (status, body)
        if decision == "approve":
            if action == "import":
                assert json.loads(body)["size"] == len(data)
            else:
                assert body == SEED
        else:
            assert data not in body and SEED not in body, "refusal returned private file bytes"
        expected = data if action == "import" and decision == "approve" else SEED
        assert guest_bytes(client, vm_id, path) == expected
        assert_approval_audit(service, client, vm_id, receipt, table="fs_events")


@pytest.mark.parametrize("action", ["import", "export"])
def test_file_caller_disconnect_cancels_original_boundary(action):
    with native_approval_session() as (service, client, api, vm_id, _):
        path = prepare(service, client, vm_id, action)
        data = uuid.uuid4().hex.encode()
        with ThreadPoolExecutor(max_workers=1) as executor:
            original = executor.submit(
                original_file_operation, client, vm_id, action, data, timeout=3
            )
            listing = pending_original_operation(api, vm_id, original, timeout=2)
            assert guest_bytes(client, vm_id, path) == SEED
            with pytest.raises(ConnectionError):
                original.result(timeout=10)
        receipt = assert_original_cancelled(api, vm_id, listing)
        assert guest_bytes(client, vm_id, path) == SEED
        assert_approval_audit(service, client, vm_id, receipt, cancelled=True, table="fs_events")
