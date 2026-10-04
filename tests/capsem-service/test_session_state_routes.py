"""Public session-state route contract for stale and incompatible VMs."""

from __future__ import annotations

from pathlib import Path

from helpers.persistent_registry import registry_entry, write_registry
from helpers.service import ServiceInstance

DEFUNCT_ID = "33333333-3333-4333-8333-333333333333"
INCOMPATIBLE_ID = "44444444-4444-4444-8444-444444444444"
DEFUNCT_NAME = "stale-overlay"
INCOMPATIBLE_NAME = "missing-overlay"


def _curl_json_with_status(service: ServiceInstance, method: str, path: str, body=None):
    return service.client().call_json(method, path, body, timeout=30)


def _row(listing: dict, vm_id: str) -> dict:
    matches = [row for row in listing["sandboxes"] if row["id"] == vm_id]
    assert len(matches) == 1, f"expected one row for {vm_id}, got {matches}"
    return matches[0]


def _assert_not_resumable(row: dict, status: str):
    assert row["status"] == status
    assert row["persistent"] is True
    assert row["can_resume"] is False
    assert row["available_actions"] == ["delete"]
    assert "start" not in row["available_actions"]
    assert "resume" not in row["available_actions"]


def test_defunct_overlayfs_session_is_non_resumable_and_purgeable():
    svc = ServiceInstance()
    try:
        last_error = (
            "FATAL: overlayfs mount failed: Stale file handle\n"
            "Kernel panic - not syncing: Attempted to kill init"
        )
        defunct = registry_entry(
            svc.tmp_dir,
            DEFUNCT_ID,
            DEFUNCT_NAME,
            defunct=False,
            last_error=None,
        )
        Path(defunct["session_dir"], "process.log").write_text("boot died before ready\n")
        Path(defunct["session_dir"], "serial.log").write_text(last_error)
        # A persistent VM whose system overlay is gone cannot boot as recorded.
        incompatible = registry_entry(svc.tmp_dir, INCOMPATIBLE_ID, INCOMPATIBLE_NAME, overlay=False)
        write_registry(svc.tmp_dir, [defunct, incompatible])

        svc.start()
        client = svc.client()

        listing = client.get("/vms/list")
        defunct_row = _row(listing, DEFUNCT_ID)
        assert defunct_row["name"] == DEFUNCT_NAME
        _assert_not_resumable(defunct_row, "Defunct")
        assert "Stale file handle" in defunct_row["last_error"]
        assert "resume_blocked_reason" not in defunct_row

        info = client.get(f"/vms/{DEFUNCT_ID}/info")
        _assert_not_resumable(info, "Defunct")
        assert "Kernel panic" in info["last_error"]
        assert info["id"] == DEFUNCT_ID
        assert info["name"] == DEFUNCT_NAME

        status = client.get(f"/vms/{DEFUNCT_ID}/status")
        _assert_not_resumable(status, "Defunct")
        assert "pid" not in status
        assert "Stale file handle" in status["last_error"]

        http_status, error = _curl_json_with_status(
            svc, "POST", f"/vms/{DEFUNCT_ID}/resume", {}
        )
        assert http_status >= 400
        assert "resume failed" in error["error"]
        assert "Stale file handle" in error["error"]

        incompatible_row = _row(client.get("/vms/list"), INCOMPATIBLE_ID)
        assert incompatible_row["name"] == INCOMPATIBLE_NAME
        _assert_not_resumable(incompatible_row, "Incompatible")
        assert "system overlay rootfs.img unavailable" in incompatible_row["resume_blocked_reason"]
        assert "last_error" not in incompatible_row

        incompatible_status = client.get(f"/vms/{INCOMPATIBLE_ID}/status")
        _assert_not_resumable(incompatible_status, "Incompatible")
        assert "system overlay rootfs.img unavailable" in incompatible_status["resume_blocked_reason"]
        assert incompatible_status.get("last_error") is None

        purge = client.post("/purge", {})
        assert purge["persistent_purged"] == 1
        assert purge["purged"] == 1

        listing_after_purge = client.get("/vms/list")
        assert not [row for row in listing_after_purge["sandboxes"] if row["id"] == DEFUNCT_ID]
        assert _row(listing_after_purge, INCOMPATIBLE_ID)["status"] == "Incompatible"
    finally:
        svc.stop()
