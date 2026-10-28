"""A stopped persistent VM with a populated workspace, seeded without booting."""

from __future__ import annotations

import json
import platform
import shutil
from pathlib import Path

from capsem_builder.release.tools.release_inputs import required_digest
from capsem_builder.release.tools.stage_release_test_inputs import _active_runtime_architecture

from .constants import DEFAULT_CPUS, DEFAULT_RAM_MB

VM_ID = "f02b6a6c-a141-4411-a032-cc79912fb248"
VM_NAME = "route-workspace"
BODY_ROUTE = "/vms/{id}/bodies/{event_id}"
BODY_EVENT_ID = "88d18d157b28"
BODY_URL = f"/vms/{VM_ID}/bodies/{BODY_EVENT_ID}?max_bytes=64"
# The logical manifest names a VM boots, by registry pin key.
BOOT_ASSETS = {"kernel": "vmlinuz", "initrd": "initrd.img", "rootfs": "rootfs.erofs"}


def manifest_asset_pins(assets_dir: Path, *, architecture: str | None = None) -> dict[str, dict[str, str]]:
    """Pin the selected local asset release or manifest-owned runtime graph."""
    manifest = json.loads((assets_dir / "manifest.json").read_text())
    arch = architecture or (
        "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"
    )
    if "runtime" in manifest:
        architecture = _active_runtime_architecture(manifest, arch)
        entries = {}
        for image in architecture["images"]:
            kind = image["kind"]
            if kind not in BOOT_ASSETS or image.get("status") == "revoked":
                continue
            name = BOOT_ASSETS[kind]
            if name in entries:
                raise ValueError(f"runtime repeats boot image {kind}")
            _, digest, _ = required_digest(image, f"runtime/{arch}/{kind}")
            entries[name] = {"hash": digest}
        missing = set(BOOT_ASSETS.values()) - entries.keys()
        if missing:
            raise ValueError(f"runtime lacks boot images: {sorted(missing)}")
    else:
        release = manifest["assets"]["releases"][manifest["assets"]["current"]]
        entries = release["arches"][arch]
    return {
        key: {
            "name": name,
            "hash": "blake3:" + entries[name]["hash"].removeprefix("blake3:"),
        }
        for key, name in BOOT_ASSETS.items()
    }


def seed_stopped_workspace(run_dir: Path, assets_dir: Path, *, file_count: int = 64) -> None:
    """Seed the registry, workspace and session ledger of one stopped
    persistent VM pinned to the manifest's current assets."""
    registry = run_dir / "persistent_registry.json"
    if registry.exists():
        raise ValueError(
            "stopped workspace fixture requires an isolated empty registry"
        )
    session = run_dir / "persistent" / VM_ID
    workspace = session / "guest" / "workspace"
    workspace.mkdir(parents=True)
    for index in range(file_count):
        (workspace / f"unchanged-{index:03}.txt").write_text("unchanged\n" * 32)
    (workspace / "modified.txt").write_text("after\n")
    (workspace / "created.txt").write_text("created\n")
    fixture = Path(__file__).resolve().parents[1] / "fixtures" / "session"
    database = Path(shutil.copy2(fixture / "test.db", session / "session.db"))
    database.chmod(0o600)
    archive_lock = Path(shutil.copy2(
        fixture / "test.db-archive.lock", session / "session.db-archive.lock"
    ))
    archive_lock.chmod(0o600)
    bodies = shutil.copytree(fixture / "test.bodies", session / "session.bodies")
    bodies.chmod(0o700)
    for path in bodies.rglob("*"):
        path.chmod(0o700 if path.is_dir() else 0o600)
    entry = {
        "id": VM_ID,
        "name": VM_NAME,
        "asset_pins": manifest_asset_pins(assets_dir),
        "ram_mb": DEFAULT_RAM_MB,
        "cpus": DEFAULT_CPUS,
        "base_version": "0.0.0-benchmark",
        "created_at": "2026-09-10T00:00:00Z",
        "session_dir": str(session),
        "defunct": False,
    }
    registry.write_text(json.dumps({"vms": {VM_NAME: entry}}))


def assert_event_bodies(payload: dict) -> None:
    """The measured route must read authenticated bytes, not a fast empty result."""
    assert payload["event_id"] == BODY_EVENT_ID, payload
    assert {body["direction"] for body in payload["bodies"]} == {
        "request",
        "response",
    }, payload
    assert all(
        body["content"] and body["truncated_for_transport"]
        for body in payload["bodies"]
    ), payload
