"""Public session-state route contract for stale and incompatible VMs."""

from __future__ import annotations

import json
import subprocess
import tomllib
from pathlib import Path

from helpers.service import ServiceInstance, materialize_test_profiles

DEFUNCT_ID = "33333333-3333-4333-8333-333333333333"
DRIFT_ID = "44444444-4444-4444-8444-444444444444"
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
        cmd += ["-d", json.dumps(body)]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    assert result.returncode == 0, result.stderr
    raw, status = result.stdout.rsplit("\n__STATUS__", 1)
    return int(status), json.loads(raw) if raw.strip() else None


def _profile_contract(tmp_dir: Path):
    profiles_dir = materialize_test_profiles(tmp_dir)
    profile_path = profiles_dir / "code" / "profile.toml"
    profile = tomllib.loads(profile_path.read_text())
    arch = "arm64" if __import__("platform").machine() == "arm64" else "x86_64"
    assets = profile["assets"]["arch"][arch]
    return {
        "revision": profile["revision"],
        "pins": {
            "kernel": {
                "name": assets["kernel"]["name"],
                "hash": assets["kernel"]["hash"],
            },
            "initrd": {
                "name": assets["initrd"]["name"],
                "hash": assets["initrd"]["hash"],
            },
            "rootfs": {
                "name": assets["rootfs"]["name"],
                "hash": assets["rootfs"]["hash"],
            },
        },
    }


def _entry(vm_id: str, name: str, tmp_dir: Path, contract: dict, **overrides):
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


def _write_registry(tmp_dir: Path, entries: list[dict]):
    (tmp_dir / "persistent_registry.json").write_text(
        json.dumps({"vms": {entry["name"]: entry for entry in entries}}, indent=2)
    )


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
        contract = _profile_contract(svc.tmp_dir)
        last_error = (
            "FATAL: overlayfs mount failed: Stale file handle\n"
            "Kernel panic - not syncing: Attempted to kill init"
        )
        defunct = _entry(
            DEFUNCT_ID,
            DEFUNCT_NAME,
            svc.tmp_dir,
            contract,
            defunct=False,
            last_error=None,
        )
        Path(defunct["session_dir"], "process.log").write_text("boot died before ready\n")
        Path(defunct["session_dir"], "serial.log").write_text(last_error)
        incompatible = _entry(DRIFT_ID, DRIFT_NAME, svc.tmp_dir, contract)
        _write_registry(svc.tmp_dir, [defunct, incompatible])

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

        drift_row = _row(client.get("/vms/list"), DRIFT_ID)
        assert drift_row["name"] == DRIFT_NAME
        _assert_not_resumable(drift_row, "Incompatible")
        assert "payload hash mismatch" in drift_row["resume_blocked_reason"]
        assert "last_error" not in drift_row

        drift_status = client.get(f"/vms/{DRIFT_ID}/status")
        _assert_not_resumable(drift_status, "Incompatible")
        assert "payload hash mismatch" in drift_status["resume_blocked_reason"]
        assert drift_status.get("last_error") is None

        purge = client.post("/purge", {})
        assert purge["persistent_purged"] == 1
        assert purge["purged"] == 1

        listing_after_purge = client.get("/vms/list")
        assert not [row for row in listing_after_purge["sandboxes"] if row["id"] == DEFUNCT_ID]
        assert _row(listing_after_purge, DRIFT_ID)["status"] == "Incompatible"
    finally:
        svc.stop()
