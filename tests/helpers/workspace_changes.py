"""Stopped workspace fixture for real checkpoint comparison route measurements."""

from __future__ import annotations

import json
import platform
import tomllib
from pathlib import Path

from .constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB

VM_ID = "f02b6a6c-a141-4411-a032-cc79912fb248"
CHANGES_ROUTE = "/vms/{id}/changes"
CHANGES_URL = f"/vms/{VM_ID}/changes?checkpoint=cp-10"


def seed_workspace_changes(run_dir: Path, profiles_dir: Path) -> None:
    """Seed registry and snapshot storage without booting or mocking a route."""
    registry = run_dir / "persistent_registry.json"
    if registry.exists():
        raise ValueError("workspace benchmark requires an isolated empty registry")
    session = run_dir / "persistent" / VM_ID
    current = session / "guest" / "workspace"
    snapshot = session / "auto_snapshots" / "10"
    before = snapshot / "workspace"
    for directory in (current, before):
        directory.mkdir(parents=True)
        for index in range(64):
            (directory / f"unchanged-{index:03}.txt").write_text("unchanged\n" * 32)
    (before / "modified.txt").write_text("before\n")
    (current / "modified.txt").write_text("after\n")
    (before / "deleted.txt").write_text("deleted\n")
    (current / "created.txt").write_text("created\n")
    (snapshot / "metadata.json").write_text(json.dumps({
        "slot": 10, "timestamp": "2026-09-10T00:00:00Z", "epoch_secs": 1788998400,
        "epoch_millis": 1788998400000, "origin": "manual", "name": "baseline", "hash": None,
    }))
    profile = tomllib.loads((profiles_dir / CODE_PROFILE_ID / "profile.toml").read_text())
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"
    assets = profile["assets"]["arch"][arch]
    entry = {
        "id": VM_ID, "name": "route-workspace", "profile_id": CODE_PROFILE_ID,
        "profile_revision": profile["revision"], "profile_payload_hash": "blake3:" + "3" * 64,
        "asset_pins": {key: {field: assets[key][field] for field in ("name", "hash")}
                       for key in ("kernel", "initrd", "rootfs")},
        "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "base_version": "0.0.0-benchmark",
        "created_at": "2026-09-10T00:00:00Z", "session_dir": str(session), "defunct": False,
    }
    registry.write_text(json.dumps({"vms": {"route-workspace": entry}}))


def assert_workspace_changes(payload: dict) -> None:
    """A quick error or empty comparison must never become a timing sample."""
    assert payload["checkpoint"] == "cp-10", payload
    assert payload["total"] == 3 and payload["has_more"] is False, payload
    assert {(row["path"], row["kind"]) for row in payload["changes"]} == {
        ("created.txt", "created"), ("deleted.txt", "deleted"), ("modified.txt", "modified"),
    }, payload
