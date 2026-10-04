"""VM profiles were removed with a clean break (google/capsem#289).

A client that still names a profile is refused rather than silently given the
default VM, and a persistent VM created from a profile is refused at every
entry point -- resume, fork, clone -- with an error that names the profile and
says to delete and recreate it. Nothing adapts the old shape.
"""

from __future__ import annotations

import json
from typing import Any

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.persistent_registry import registry_entry, write_registry
from helpers.service import ServiceInstance

pytestmark = pytest.mark.integration

LEGACY_ID = "88888888-8888-4888-8888-888888888888"
LEGACY_NAME = "made-from-a-profile"


def _assert_names_the_profile(message: str) -> None:
    assert f"VM '{LEGACY_NAME}' was created from the 'co-work' profile" in message, message
    assert "profiles no longer exist" in message, message
    assert "delete it and create a new VM" in message, message


def _vm_ids(client: Any) -> set[str]:
    return {row["id"] for row in client.get("/vms/list")["sandboxes"]}


@pytest.fixture
def legacy_service():
    service = ServiceInstance()
    write_registry(
        service.tmp_dir,
        [registry_entry(service.tmp_dir, LEGACY_ID, LEGACY_NAME, profile_id="co-work")],
    )
    service.start()
    try:
        yield service
    finally:
        service.stop()


def test_create_and_run_refuse_a_profile_id(legacy_service: ServiceInstance) -> None:
    client = legacy_service.client()
    before = _vm_ids(client)
    for path, body in (
        ("/vms/create", {"profile_id": "code", "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS}),
        ("/vms/create", {"name": "named-profile", "profile_id": "code"}),
        ("/run", {"command": "true", "profile_id": "code"}),
    ):
        status, payload = client.call_json("POST", path, body, timeout=30)
        assert status in {400, 422}, (path, status, payload)
        assert "unknown field `profile_id`" in json.dumps(payload), (path, payload)
    assert _vm_ids(client) == before


def test_a_vm_created_from_a_profile_is_refused_by_name(legacy_service: ServiceInstance) -> None:
    client = legacy_service.client()

    for row in (
        next(row for row in client.get("/vms/list")["sandboxes"] if row["id"] == LEGACY_ID),
        client.get(f"/vms/{LEGACY_ID}/info"),
        client.get(f"/vms/{LEGACY_ID}/status"),
    ):
        assert row["status"] == "Incompatible", row
        assert row["can_resume"] is False
        assert row["available_actions"] == ["delete"]
        assert "profile_id" not in row
        _assert_names_the_profile(row["resume_blocked_reason"])

    for method, path, body in (
        ("POST", f"/vms/{LEGACY_ID}/resume", {}),
        ("POST", f"/vms/{LEGACY_ID}/start", {}),
        ("POST", f"/vms/{LEGACY_ID}/fork", {"name": "fork-of-legacy"}),
        ("POST", "/vms/create", {"name": "clone-of-legacy", "from": LEGACY_NAME, "persistent": True}),
    ):
        status, payload = client.call_json(method, path, body, timeout=60)
        assert status >= 400, (path, status, payload)
        _assert_names_the_profile(payload["error"])
    assert _vm_ids(client) == {LEGACY_ID}

    # The refusal is not laundered: the entry keeps the profile it was made from.
    registry = json.loads((legacy_service.tmp_dir / "persistent_registry.json").read_text())
    assert registry["vms"][LEGACY_NAME]["profile_id"] == "co-work"

    assert client.delete(f"/vms/{LEGACY_ID}/delete") == {"success": True}
    assert LEGACY_ID not in _vm_ids(client)
