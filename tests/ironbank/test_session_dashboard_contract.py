"""Ironbank session dashboard contract.

The UI and TUI must be able to render sessions from route-owned truth alone.
This black-box test starts the service, seeds only public persistent session
state, and verifies the same JSON shape the dashboard consumes.
"""

from __future__ import annotations

import uuid
from pathlib import Path
from typing import Any

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.persistent_registry import registry_entry, write_registry
from helpers.service import ServiceInstance

DEFUNCT_ID = "77777777-7777-4777-8777-777777777777"
INCOMPATIBLE_ID = "88888888-8888-4888-8888-888888888888"
DEFUNCT_NAME = "stale-overlay"
INCOMPATIBLE_NAME = "missing-overlay"


def _curl_json_with_status(service: ServiceInstance, method: str, path: str, body=None):
    return service.client().call_json(method, path, body, timeout=30)


def _row(payload: dict[str, Any], session_id: str) -> dict[str, Any]:
    rows = [row for row in payload["sandboxes"] if row["id"] == session_id]
    assert len(rows) == 1, (session_id, payload)
    return rows[0]


def _assert_delete_only(row: dict[str, Any], *, session_id: str, name: str, status: str) -> None:
    assert row["id"] == session_id
    assert row["name"] == name
    assert row["status"] == status
    assert row["persistent"] is True
    assert row["can_resume"] is False
    assert row["available_actions"] == ["delete"]
    for forbidden in ("start", "resume", "pause", "stop", "fork"):
        assert forbidden not in row["available_actions"]


def test_session_dashboard_routes_are_delete_only_for_broken_sessions() -> None:
    service = ServiceInstance()
    try:
        defunct = registry_entry(
            service.tmp_dir,
            DEFUNCT_ID,
            DEFUNCT_NAME,
            ram_mb=DEFAULT_RAM_MB,
            cpus=DEFAULT_CPUS,
        )
        Path(defunct["session_dir"], "serial.log").write_text(
            "overlayfs mount failed: Stale file handle\nKernel panic - not syncing",
            encoding="utf-8",
        )
        # A persistent VM whose system overlay is gone cannot boot as recorded.
        incompatible = registry_entry(
            service.tmp_dir,
            INCOMPATIBLE_ID,
            INCOMPATIBLE_NAME,
            ram_mb=DEFAULT_RAM_MB,
            cpus=DEFAULT_CPUS, overlay=False,
        )
        write_registry(service.tmp_dir, [defunct, incompatible])

        service.start()
        client = service.client()

        listing = client.get("/vms/list", timeout=30)
        assert "sandboxes" in listing
        defunct_row = _row(listing, DEFUNCT_ID)
        incompatible_row = _row(listing, INCOMPATIBLE_ID)
        _assert_delete_only(defunct_row, session_id=DEFUNCT_ID, name=DEFUNCT_NAME, status="Defunct")
        _assert_delete_only(
            incompatible_row,
            session_id=INCOMPATIBLE_ID,
            name=INCOMPATIBLE_NAME,
            status="Incompatible",
        )
        assert "Stale file handle" in defunct_row["last_error"]
        assert "system overlay rootfs.img unavailable" in incompatible_row["resume_blocked_reason"]

        for session_id, name, status in (
            (DEFUNCT_ID, DEFUNCT_NAME, "Defunct"),
            (INCOMPATIBLE_ID, INCOMPATIBLE_NAME, "Incompatible"),
        ):
            _assert_delete_only(
                client.get(f"/vms/{session_id}/status", timeout=30),
                session_id=session_id,
                name=name,
                status=status,
            )
            _assert_delete_only(
                client.get(f"/vms/{session_id}/info", timeout=30),
                session_id=session_id,
                name=name,
                status=status,
            )
            http_status, error = _curl_json_with_status(
                service,
                "POST",
                f"/vms/{session_id}/resume",
                {},
            )
            assert http_status >= 400
            assert "resume" in error["error"].lower()

        purge = client.post("/purge", {}, timeout=30)
        assert purge["persistent_purged"] == 1
        assert purge["purged"] == 1
        after_purge = client.get("/vms/list", timeout=30)
        assert DEFUNCT_ID not in {row["id"] for row in after_purge["sandboxes"]}
        assert _row(after_purge, INCOMPATIBLE_ID)["status"] == "Incompatible"

        assert client.delete(f"/vms/{INCOMPATIBLE_ID}/delete", timeout=30) == {"success": True}
        after_delete = client.get("/vms/list", timeout=30)
        assert INCOMPATIBLE_ID not in {row["id"] for row in after_delete["sandboxes"]}
    finally:
        service.stop()


def test_session_dashboard_create_names_are_vm_counted_not_tmp() -> None:
    service = ServiceInstance()
    created: list[str] = []
    try:
        service.start()
        client = service.client()

        for expected_name in ("vm-1", "vm-2"):
            response = client.post(
                "/vms/create",
                {
                    "ram_mb": DEFAULT_RAM_MB,
                    "cpus": DEFAULT_CPUS,
                },
                timeout=30,
            )
            session_id = response["id"]
            created.append(session_id)
            uuid.UUID(session_id)
            assert session_id != expected_name
            assert response["name"] == expected_name
            assert not session_id.startswith("tmp-")
            status = client.get(f"/vms/{session_id}/status", timeout=30)
            assert status["id"] == session_id
            assert status["name"] == expected_name
            assert set(status["available_actions"]) >= {"fork", "delete"}
            info = client.get(f"/vms/{session_id}/info", timeout=30)
            assert info["id"] == session_id
            assert info["name"] == expected_name

        listing = client.get("/vms/list", timeout=30)
        listed = {row["id"]: row for row in listing["sandboxes"]}
        assert set(created) <= listed.keys()
        assert [listed[session_id]["name"] for session_id in created] == ["vm-1", "vm-2"]
    finally:
        if service.proc is not None:
            client = service.client()
            for session_id in created:
                client.delete(f"/vms/{session_id}/delete", timeout=30)
        service.stop()
