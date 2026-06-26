"""Ironbank session dashboard contract.

The UI and TUI must be able to render sessions from route-owned truth alone.
This black-box test starts the service, seeds only public persistent session
state, and verifies the same JSON shape the dashboard consumes.
"""

from __future__ import annotations

import json
import platform
import subprocess
import tomllib
from pathlib import Path
from typing import Any

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, materialize_test_profiles


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
    profile = tomllib.loads((profiles_dir / CODE_PROFILE_ID / "profile.toml").read_text())
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"
    assets = profile["assets"]["arch"][arch]
    return {
        "revision": profile["revision"],
        "pins": {
            "kernel": {"name": assets["kernel"]["name"], "hash": assets["kernel"]["hash"]},
            "initrd": {"name": assets["initrd"]["name"], "hash": assets["initrd"]["hash"]},
            "rootfs": {"name": assets["rootfs"]["name"], "hash": assets["rootfs"]["hash"]},
        },
    }


def _registry_entry(name: str, tmp_dir: Path, contract: dict[str, Any], **overrides):
    session_dir = tmp_dir / "persistent" / name
    session_dir.mkdir(parents=True, exist_ok=True)
    data = {
        "name": name,
        "profile_id": CODE_PROFILE_ID,
        "profile_revision": contract["revision"],
        "profile_payload_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "asset_pins": contract["pins"],
        "ram_mb": DEFAULT_RAM_MB,
        "cpus": DEFAULT_CPUS,
        "base_version": "0.0.0-ironbank",
        "created_at": "2026-06-17T00:00:00Z",
        "session_dir": str(session_dir),
        "defunct": False,
    }
    data.update(overrides)
    return data


def _write_registry(tmp_dir: Path, entries: list[dict[str, Any]]) -> None:
    (tmp_dir / "persistent_registry.json").write_text(
        json.dumps({"vms": {entry["name"]: entry for entry in entries}}, indent=2),
        encoding="utf-8",
    )


def _row(payload: dict[str, Any], session_id: str) -> dict[str, Any]:
    rows = [row for row in payload["sandboxes"] if row["id"] == session_id]
    assert len(rows) == 1, (session_id, payload)
    return rows[0]


def _assert_delete_only(row: dict[str, Any], *, session_id: str, status: str) -> None:
    assert row["id"] == session_id
    assert row["status"] == status
    if "profile_id" in row:
        assert row["profile_id"] == CODE_PROFILE_ID
    assert row["persistent"] is True
    assert row["can_resume"] is False
    assert row["available_actions"] == ["delete"]
    for forbidden in ("start", "resume", "pause", "stop", "fork"):
        assert forbidden not in row["available_actions"]


def test_session_dashboard_routes_are_profile_owned_and_delete_only_for_broken_sessions() -> None:
    service = ServiceInstance()
    try:
        contract = _profile_contract(service.tmp_dir)
        defunct = _registry_entry("code-stale-overlay", service.tmp_dir, contract)
        Path(defunct["session_dir"], "serial.log").write_text(
            "overlayfs mount failed: Stale file handle\nKernel panic - not syncing",
            encoding="utf-8",
        )
        incompatible = _registry_entry(
            "code-payload-drift",
            service.tmp_dir,
            contract,
            profile_payload_hash="blake3:0000000000000000000000000000000000000000000000000000000000000000",
        )
        _write_registry(service.tmp_dir, [defunct, incompatible])

        service.start()
        client = service.client()

        profiles = client.get("/profiles/list", timeout=30)
        by_id = {profile["id"]: profile for profile in profiles["profiles"]}
        assert {"code", "co-work"} <= by_id.keys()
        assert by_id["code"]["name"] == "Code"
        assert by_id["code"]["description"] == "Optimized for coding and long-running agents."
        assert by_id["code"]["availability"]["shell"] is True
        assert by_id["co-work"]["availability"]["shell"] is True
        assert all("policy" not in profile for profile in by_id.values())

        listing = client.get("/vms/list", timeout=30)
        assert "sandboxes" in listing
        defunct_row = _row(listing, "code-stale-overlay")
        incompatible_row = _row(listing, "code-payload-drift")
        assert defunct_row["profile_id"] == CODE_PROFILE_ID
        assert incompatible_row["profile_id"] == CODE_PROFILE_ID
        _assert_delete_only(defunct_row, session_id="code-stale-overlay", status="Defunct")
        _assert_delete_only(
            incompatible_row,
            session_id="code-payload-drift",
            status="Incompatible",
        )
        assert "Stale file handle" in defunct_row["last_error"]
        assert "payload hash mismatch" in incompatible_row["resume_blocked_reason"]

        for session_id, status in (
            ("code-stale-overlay", "Defunct"),
            ("code-payload-drift", "Incompatible"),
        ):
            _assert_delete_only(
                client.get(f"/vms/{session_id}/status", timeout=30),
                session_id=session_id,
                status=status,
            )
            _assert_delete_only(
                client.get(f"/vms/{session_id}/info", timeout=30),
                session_id=session_id,
                status=status,
            )
            assert client.get(f"/vms/{session_id}/info", timeout=30)["profile_id"] == CODE_PROFILE_ID
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
        assert "code-stale-overlay" not in {row["id"] for row in after_purge["sandboxes"]}
        assert _row(after_purge, "code-payload-drift")["status"] == "Incompatible"

        assert client.delete("/vms/code-payload-drift/delete", timeout=30) == {"success": True}
        after_delete = client.get("/vms/list", timeout=30)
        assert "code-payload-drift" not in {row["id"] for row in after_delete["sandboxes"]}
    finally:
        service.stop()


def test_session_dashboard_create_names_are_profile_scoped_not_tmp() -> None:
    service = ServiceInstance()
    created: list[str] = []
    try:
        service.start()
        client = service.client()

        for expected_id in ("code-1", "code-2"):
            response = client.post(
                "/vms/create",
                {
                    "profile_id": CODE_PROFILE_ID,
                    "ram_mb": DEFAULT_RAM_MB,
                    "cpus": DEFAULT_CPUS,
                },
                timeout=30,
            )
            session_id = response["id"]
            created.append(session_id)
            assert session_id == expected_id
            assert not session_id.startswith("tmp-")
            status = client.get(f"/vms/{session_id}/status", timeout=30)
            assert status["id"] == session_id
            assert set(status["available_actions"]) >= {"fork", "delete"}
            info = client.get(f"/vms/{session_id}/info", timeout=30)
            assert info["id"] == session_id
            assert info["profile_id"] == CODE_PROFILE_ID

        listing = client.get("/vms/list", timeout=30)
        listed = {row["id"]: row for row in listing["sandboxes"]}
        assert set(created) <= listed.keys()
        assert [listed[session_id]["profile_id"] for session_id in created] == [
            CODE_PROFILE_ID,
            CODE_PROFILE_ID,
        ]
    finally:
        if service.proc is not None:
            client = service.client()
            for session_id in created:
                client.delete(f"/vms/{session_id}/delete", timeout=30)
        service.stop()
