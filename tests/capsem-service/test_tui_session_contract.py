"""TUI-facing session route contract.

The TUI reflects route-owned facts only. Broken or incompatible sessions must
never look resumable, and launchable profiles must come from profile routes.
"""

from __future__ import annotations

import json
import platform
import subprocess
import tomllib
from pathlib import Path
from typing import Any

from helpers.service import ServiceInstance, materialize_test_profiles

DEFUNCT_ID = "55555555-5555-4555-8555-555555555555"
DRIFT_ID = "66666666-6666-4666-8666-666666666666"
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
        "profile_payload_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
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


def test_tui_session_routes_expose_profile_truth_and_delete_only_broken_sessions() -> None:
    service = ServiceInstance()
    try:
        contract = _profile_contract(service.tmp_dir)
        defunct = _registry_entry(DEFUNCT_ID, DEFUNCT_NAME, service.tmp_dir, contract)
        Path(defunct["session_dir"], "serial.log").write_text(
            "overlayfs mount failed: Stale file handle\nKernel panic - not syncing"
        )
        incompatible = _registry_entry(DRIFT_ID, DRIFT_NAME, service.tmp_dir, contract)
        _write_registry(service.tmp_dir, [defunct, incompatible])

        service.start()
        client = service.client()

        profiles = client.get("/profiles/list")
        by_id = {profile["id"]: profile for profile in profiles["profiles"]}
        assert {"code", "co-work"} <= by_id.keys()
        assert by_id["code"]["name"] == "Code"
        assert by_id["code"]["description"] == "Optimized for coding and long-running agents."
        assert by_id["code"]["availability"]["shell"] is True
        assert by_id["co-work"]["availability"]["shell"] is True

        listing = client.get("/vms/list")
        defunct_row = _row(listing, DEFUNCT_ID)
        incompatible_row = _row(listing, DRIFT_ID)
        _assert_delete_only(defunct_row, session_id=DEFUNCT_ID, name=DEFUNCT_NAME, status="Defunct")
        _assert_delete_only(
            incompatible_row,
            session_id=DRIFT_ID,
            name=DRIFT_NAME,
            status="Incompatible",
        )
        assert "Stale file handle" in defunct_row["last_error"]
        assert "payload hash mismatch" in incompatible_row["resume_blocked_reason"]

        for session_id in (DEFUNCT_ID, DRIFT_ID):
            status, payload = _curl_json_with_status(service, "POST", f"/vms/{session_id}/resume", {})
            assert status >= 400
            assert "resume" in payload["error"].lower()

        purge = client.post("/purge", {})
        assert purge["persistent_purged"] == 1
        assert purge["purged"] == 1
    finally:
        service.stop()
