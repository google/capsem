"""A stopped persistent VM with a populated workspace, seeded without booting."""

from __future__ import annotations

import json
import platform
import shutil
import tomllib
from pathlib import Path

from .constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB

VM_ID = "f02b6a6c-a141-4411-a032-cc79912fb248"
VM_NAME = "route-workspace"
BODY_ROUTE = "/vms/{id}/bodies/{event_id}"
BODY_EVENT_ID = "88d18d157b28"
BODY_URL = f"/vms/{VM_ID}/bodies/{BODY_EVENT_ID}?max_bytes=64"


def seed_stopped_workspace(run_dir: Path, profiles_dir: Path) -> None:
    """Seed the registry, workspace and session ledger of one stopped
    persistent VM."""
    registry = run_dir / "persistent_registry.json"
    if registry.exists():
        raise ValueError("stopped workspace fixture requires an isolated empty registry")
    session = run_dir / "persistent" / VM_ID
    workspace = session / "guest" / "workspace"
    workspace.mkdir(parents=True)
    for index in range(64):
        (workspace / f"unchanged-{index:03}.txt").write_text("unchanged\n" * 32)
    (workspace / "modified.txt").write_text("after\n")
    (workspace / "created.txt").write_text("created\n")
    fixture = Path(__file__).resolve().parents[1] / "fixtures" / "session"
    shutil.copy2(fixture / "test.db", session / "session.db")
    shutil.copy2(fixture / "test.db-archive.lock", session / "session.db-archive.lock")
    shutil.copytree(fixture / "test.bodies", session / "session.bodies")
    profile = tomllib.loads((profiles_dir / CODE_PROFILE_ID / "profile.toml").read_text())
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"
    assets = profile["assets"]["arch"][arch]
    entry = {
        "id": VM_ID, "name": VM_NAME, "profile_id": CODE_PROFILE_ID,
        "profile_revision": profile["revision"], "profile_payload_hash": "blake3:" + "3" * 64,
        "asset_pins": {key: {field: assets[key][field] for field in ("name", "hash")}
                       for key in ("kernel", "initrd", "rootfs")},
        "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "base_version": "0.0.0-benchmark",
        "created_at": "2026-09-10T00:00:00Z", "session_dir": str(session), "defunct": False,
    }
    registry.write_text(json.dumps({"vms": {VM_NAME: entry}}))


def assert_event_bodies(payload: dict) -> None:
    """The measured route must read authenticated bytes, not a fast empty result."""
    assert payload["event_id"] == BODY_EVENT_ID, payload
    assert {body["direction"] for body in payload["bodies"]} == {"request", "response"}, payload
    assert all(body["content"] and body["truncated_for_transport"] for body in payload["bodies"]), payload
