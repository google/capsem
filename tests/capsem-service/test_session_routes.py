"""Session route contract for UI/TUI session dashboards.

The dashboard must reflect route-owned lifecycle truth. Defunct and
incompatible sessions are not resumable, not openable, and expose delete only.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from helpers.persistent_registry import registry_entry, write_registry
from helpers.service import ServiceInstance

DEFUNCT_ID = "11111111-1111-4111-8111-111111111111"
INCOMPATIBLE_ID = "22222222-2222-4222-8222-222222222222"
MISSING_ASSET_ID = "77777777-7777-4777-8777-777777777777"
DEFUNCT_NAME = "stale-overlay"
INCOMPATIBLE_NAME = "missing-overlay"
MISSING_ASSET_NAME = "missing-asset-pin"


def _curl_json_with_status(service: ServiceInstance, method: str, path: str, body=None):
    return service.client().call_json(method, path, body, timeout=30)


def _row(listing: dict[str, Any], session_id: str) -> dict[str, Any]:
    matches = [row for row in listing["sandboxes"] if row["id"] == session_id]
    assert len(matches) == 1, (session_id, listing)
    return matches[0]


def _assert_delete_only_session(
    payload: dict[str, Any], *, session_id: str, name: str, status: str
) -> None:
    assert payload["id"] == session_id
    if "name" in payload:
        assert payload["name"] == name
    assert payload["status"] == status
    assert payload["persistent"] is True
    assert payload["can_resume"] is False
    assert payload["available_actions"] == ["delete"]
    assert "start" not in payload["available_actions"]
    assert "resume" not in payload["available_actions"]
    assert "fork" not in payload["available_actions"]


def test_session_routes_make_defunct_and_incompatible_sessions_delete_only() -> None:
    service = ServiceInstance()
    try:
        stale_log = "overlayfs mount failed: Stale file handle\nKernel panic - not syncing"
        defunct = registry_entry(service.tmp_dir, DEFUNCT_ID, DEFUNCT_NAME)
        Path(defunct["session_dir"], "process.log").write_text("boot failed\n")
        Path(defunct["session_dir"], "serial.log").write_text(stale_log)
        # A persistent VM whose system overlay is gone cannot boot as recorded.
        incompatible = registry_entry(service.tmp_dir, INCOMPATIBLE_ID, INCOMPATIBLE_NAME, overlay=False)
        write_registry(service.tmp_dir, [defunct, incompatible])

        service.start()
        client = service.client()

        listing = client.get("/vms/list")
        defunct_row = _row(listing, DEFUNCT_ID)
        incompatible_row = _row(listing, INCOMPATIBLE_ID)
        _assert_delete_only_session(
            defunct_row, session_id=DEFUNCT_ID, name=DEFUNCT_NAME, status="Defunct"
        )
        _assert_delete_only_session(
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
            _assert_delete_only_session(
                client.get(f"/vms/{session_id}/status"),
                session_id=session_id,
                name=name,
                status=status,
            )
            _assert_delete_only_session(
                client.get(f"/vms/{session_id}/info"),
                session_id=session_id,
                name=name,
                status=status,
            )
            http_status, error = _curl_json_with_status(
                service, "POST", f"/vms/{session_id}/resume", {}
            )
            assert http_status >= 400
            assert "resume" in error["error"].lower()

        assert client.delete(f"/vms/{DEFUNCT_ID}/delete") == {"success": True}
        assert client.delete(f"/vms/{INCOMPATIBLE_ID}/delete") == {"success": True}
        listing_after_delete = client.get("/vms/list")
        assert DEFUNCT_ID not in {row["id"] for row in listing_after_delete["sandboxes"]}
        assert INCOMPATIBLE_ID not in {row["id"] for row in listing_after_delete["sandboxes"]}
    finally:
        service.stop()


def test_missing_pinned_asset_blocks_resume_but_keeps_fork_and_delete() -> None:
    service = ServiceInstance()
    try:
        entry = registry_entry(service.tmp_dir, MISSING_ASSET_ID, MISSING_ASSET_NAME)
        # Neither the hash-named nor the logical file exists in the assets dir.
        entry["asset_pins"]["rootfs"] = {"name": "rootfs-gone.erofs", "hash": "blake3:" + "f" * 64}
        write_registry(service.tmp_dir, [entry])

        service.start()
        client = service.client()

        for payload in (
            _row(client.get("/vms/list"), MISSING_ASSET_ID),
            client.get(f"/vms/{MISSING_ASSET_ID}/status"),
        ):
            assert payload["status"] == "Stopped", payload
            assert payload["can_resume"] is False
            assert payload["available_actions"] == ["fork", "delete"]
            assert "rootfs asset" in payload["resume_blocked_reason"], payload
            assert "is missing" in payload["resume_blocked_reason"], payload

        http_status, error = _curl_json_with_status(
            service, "POST", f"/vms/{MISSING_ASSET_ID}/resume", {}
        )
        assert http_status >= 400
        assert "rootfs asset" in error["error"], error
    finally:
        service.stop()
