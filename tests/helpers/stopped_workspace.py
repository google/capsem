"""A stopped persistent VM with a populated workspace, seeded without booting."""

from __future__ import annotations

import json
import platform
import shutil
from pathlib import Path

from .constants import DEFAULT_CPUS, DEFAULT_RAM_MB

VM_ID = "f02b6a6c-a141-4411-a032-cc79912fb248"
VM_NAME = "route-workspace"
BODY_ROUTE = "/vms/{id}/bodies/{event_id}"
BODY_EVENT_ID = "88d18d157b28"
BODY_URL = f"/vms/{VM_ID}/bodies/{BODY_EVENT_ID}?max_bytes=64"
# The logical manifest names a VM boots, by registry pin key.
BOOT_ASSETS = {"kernel": "vmlinuz", "initrd": "initrd.img", "rootfs": "rootfs.erofs"}


def manifest_asset_pins(assets_dir: Path) -> dict[str, dict[str, str]]:
    """The registry pins of the installed manifest's current asset release."""
    manifest = json.loads((assets_dir / "manifest.json").read_text())
    release = manifest["assets"]["releases"][manifest["assets"]["current"]]
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"
    entries = release["arches"][arch]
    return {
        key: {"name": name, "hash": "blake3:" + entries[name]["hash"].removeprefix("blake3:")}
        for key, name in BOOT_ASSETS.items()
    }


def seed_stopped_workspace(run_dir: Path, assets_dir: Path) -> None:
    """Seed the registry, workspace and session ledger of one stopped
    persistent VM pinned to the manifest's current assets."""
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
    entry = {
        "id": VM_ID, "name": VM_NAME, "asset_pins": manifest_asset_pins(assets_dir),
        "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "base_version": "0.0.0-benchmark",
        "created_at": "2026-09-10T00:00:00Z", "session_dir": str(session), "defunct": False,
    }
    registry.write_text(json.dumps({"vms": {VM_NAME: entry}}))


def assert_event_bodies(payload: dict) -> None:
    """The measured route must read authenticated bytes, not a fast empty result."""
    assert payload["event_id"] == BODY_EVENT_ID, payload
    assert {body["direction"] for body in payload["bodies"]} == {"request", "response"}, payload
    assert all(body["content"] and body["truncated_for_transport"] for body in payload["bodies"]), payload
