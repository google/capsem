"""TUI-facing session route contract.

The TUI reflects route-owned facts only. Broken or incompatible sessions must
never look resumable, and whether a new session can launch comes from the
asset status route.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from helpers.persistent_registry import registry_entry, write_registry
from helpers.service import ServiceInstance

DEFUNCT_ID = "55555555-5555-4555-8555-555555555555"
LEGACY_ID = "66666666-6666-4666-8666-666666666666"
DEFUNCT_NAME = "stale-overlay"
LEGACY_NAME = "legacy-profile"


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
    for forbidden in ("resume", "start", "pause", "stop", "fork"):
        assert forbidden not in row["available_actions"]


def test_tui_session_routes_expose_launch_truth_and_delete_only_broken_sessions() -> None:
    service = ServiceInstance()
    try:
        defunct = registry_entry(service.tmp_dir, DEFUNCT_ID, DEFUNCT_NAME)
        Path(defunct["session_dir"], "serial.log").write_text(
            "overlayfs mount failed: Stale file handle\nKernel panic - not syncing"
        )
        # An entry written before profiles were removed is incompatible.
        incompatible = registry_entry(service.tmp_dir, LEGACY_ID, LEGACY_NAME, profile_id="code")
        write_registry(service.tmp_dir, [defunct, incompatible])

        service.start()
        client = service.client()

        assets = client.get("/assets/status")
        assert isinstance(assets["ready"], bool)
        assert assets["ready"] == (not assets["errors"] and not assets["downloading"]), assets
        assert client.get("/profiles/list") is None

        listing = client.get("/vms/list")
        defunct_row = _row(listing, DEFUNCT_ID)
        incompatible_row = _row(listing, LEGACY_ID)
        _assert_delete_only(defunct_row, session_id=DEFUNCT_ID, name=DEFUNCT_NAME, status="Defunct")
        _assert_delete_only(
            incompatible_row,
            session_id=LEGACY_ID,
            name=LEGACY_NAME,
            status="Incompatible",
        )
        assert "Stale file handle" in defunct_row["last_error"]
        assert "'code' profile" in incompatible_row["resume_blocked_reason"]

        for session_id in (DEFUNCT_ID, LEGACY_ID):
            status, payload = _curl_json_with_status(service, "POST", f"/vms/{session_id}/resume", {})
            assert status >= 400
            assert "resume" in payload["error"].lower()

        purge = client.post("/purge", {})
        assert purge["persistent_purged"] == 1
        assert purge["purged"] == 1
    finally:
        service.stop()
