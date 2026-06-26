"""Session route contract for UI/TUI session dashboards.

The dashboard must reflect route-owned lifecycle truth. Defunct and
incompatible sessions are not resumable, not openable, and expose delete only.
"""

from __future__ import annotations

import json
import platform
import subprocess
import tomllib
from pathlib import Path
from typing import Any

from helpers.service import ServiceInstance, materialize_test_profiles

DEFUNCT_ID = "11111111-1111-4111-8111-111111111111"
DRIFT_ID = "22222222-2222-4222-8222-222222222222"
DEFUNCT_NAME = "code-stale-overlay"
DRIFT_NAME = "code-payload-drift"


def _curl_json_with_status(service: ServiceInstance, method: str, path: str, body=None):
    cmd = [
        "curl",
        "-s",
        "-S",
        "--unix-socket",
        str(service.uds_path),
        "-X",
        method,
        "-H",
        "Content-Type: application/json",
        "-o",
        "-",
        "-w",
        "\n__STATUS__%{http_code}",
        f"http://localhost{path}",
    ]
    if body is not None:
        cmd.extend(["-d", json.dumps(body)])
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    assert result.returncode == 0, result.stderr
    raw, status = result.stdout.rsplit("\n__STATUS__", 1)
    return int(status), json.loads(raw) if raw.strip() else None


def _profile_contract(tmp_dir: Path) -> dict[str, Any]:
    profiles_dir = materialize_test_profiles(tmp_dir)
    profile = tomllib.loads((profiles_dir / "code" / "profile.toml").read_text())
    arch = "arm64" if platform.machine() == "arm64" else "x86_64"
    assets = profile["assets"]["arch"][arch]
    return {
        "revision": profile["revision"],
        "pins": {
            "kernel": {"name": assets["kernel"]["name"], "hash": assets["kernel"]["hash"]},
            "initrd": {"name": assets["initrd"]["name"], "hash": assets["initrd"]["hash"]},
            "rootfs": {"name": assets["rootfs"]["name"], "hash": assets["rootfs"]["hash"]},
        },
    }


def _registry_entry(vm_id: str, name: str, tmp_dir: Path, contract: dict[str, Any], **overrides):
    session_dir = tmp_dir / "persistent" / vm_id
    session_dir.mkdir(parents=True, exist_ok=True)
    data = {
        "id": vm_id,
        "name": name,
        "profile_id": "code",
        "profile_revision": contract["revision"],
        "profile_payload_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
        "asset_pins": contract["pins"],
        "ram_mb": 2048,
        "cpus": 2,
        "base_version": "0.0.0-test",
        "created_at": "2026-06-16T00:00:00Z",
        "session_dir": str(session_dir),
        "defunct": False,
    }
    data.update(overrides)
    return data


def _write_registry(tmp_dir: Path, entries: list[dict[str, Any]]) -> None:
    (tmp_dir / "persistent_registry.json").write_text(
        json.dumps({"vms": {entry["name"]: entry for entry in entries}}, indent=2)
    )


def _row(listing: dict[str, Any], session_id: str) -> dict[str, Any]:
    matches = [row for row in listing["sandboxes"] if row["id"] == session_id]
    assert len(matches) == 1, (session_id, listing)
    return matches[0]


def _assert_delete_only_session(payload: dict[str, Any], *, session_id: str, name: str, status: str) -> None:
    assert payload["id"] == session_id
    if "name" in payload:
        assert payload["name"] == name
    if "profile_id" in payload:
        assert payload["profile_id"] == "code"
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
        contract = _profile_contract(service.tmp_dir)
        stale_log = "overlayfs mount failed: Stale file handle\nKernel panic - not syncing"
        defunct = _registry_entry(DEFUNCT_ID, DEFUNCT_NAME, service.tmp_dir, contract)
        Path(defunct["session_dir"], "process.log").write_text("boot failed\n")
        Path(defunct["session_dir"], "serial.log").write_text(stale_log)
        incompatible = _registry_entry(
            DRIFT_ID,
            DRIFT_NAME,
            service.tmp_dir,
            contract,
            profile_payload_hash="blake3:0000000000000000000000000000000000000000000000000000000000000000",
        )
        _write_registry(service.tmp_dir, [defunct, incompatible])

        service.start()
        client = service.client()

        listing = client.get("/vms/list")
        defunct_row = _row(listing, DEFUNCT_ID)
        incompatible_row = _row(listing, DRIFT_ID)
        _assert_delete_only_session(defunct_row, session_id=DEFUNCT_ID, name=DEFUNCT_NAME, status="Defunct")
        _assert_delete_only_session(
            incompatible_row,
            session_id=DRIFT_ID,
            name=DRIFT_NAME,
            status="Incompatible",
        )
        assert "Stale file handle" in defunct_row["last_error"]
        assert "payload hash mismatch" in incompatible_row["resume_blocked_reason"]

        for session_id, name, status in (
            (DEFUNCT_ID, DEFUNCT_NAME, "Defunct"),
            (DRIFT_ID, DRIFT_NAME, "Incompatible"),
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
            http_status, error = _curl_json_with_status(service, "POST", f"/vms/{session_id}/resume", {})
            assert http_status >= 400
            assert "resume" in error["error"].lower()

        assert client.delete(f"/vms/{DEFUNCT_ID}/delete") == {"success": True}
        assert client.delete(f"/vms/{DRIFT_ID}/delete") == {"success": True}
        listing_after_delete = client.get("/vms/list")
        assert DEFUNCT_ID not in {row["id"] for row in listing_after_delete["sandboxes"]}
        assert DRIFT_ID not in {row["id"] for row in listing_after_delete["sandboxes"]}
    finally:
        service.stop()
