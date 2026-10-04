"""Seed `persistent_registry.json` entries in the current registry shape.

An entry pins the boot assets it was created with (`asset_pins`), taken here
from the installed manifest's current release. An entry that still carries a
`profile_id` is a VM from before profiles were removed; tests write one with
`profile_id=...` in `overrides` to prove it is refused.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from .constants import ASSETS_DIR
from .stopped_workspace import manifest_asset_pins

GIB = 1024 * 1024 * 1024


def registry_entry(
    run_dir: Path,
    vm_id: str,
    name: str,
    *,
    assets_dir: Path = ASSETS_DIR,
    overlay: bool = True,
    **overrides: Any,
) -> dict[str, Any]:
    """One persistent VM's registry entry, with its session directory.

    `overlay` creates the sparse whole-GiB system overlay a stopped VM keeps,
    so the entry is judged on its pins and shape rather than on a missing disk.
    """
    session_dir = Path(run_dir) / "persistent" / vm_id
    session_dir.mkdir(parents=True, exist_ok=True)
    if overlay:
        system = session_dir / "system"
        system.mkdir(exist_ok=True)
        with (system / "rootfs.img").open("wb") as image:
            image.truncate(GIB)
    entry = {
        "id": vm_id,
        "name": name,
        "asset_pins": manifest_asset_pins(assets_dir),
        "ram_mb": 2048,
        "cpus": 2,
        "base_version": "0.0.0-test",
        "created_at": "2026-06-16T00:00:00Z",
        "session_dir": str(session_dir),
        "defunct": False,
    }
    entry.update(overrides)
    return entry


def write_registry(run_dir: Path, entries: list[dict[str, Any]]) -> None:
    (Path(run_dir) / "persistent_registry.json").write_text(
        json.dumps({"vms": {entry["name"]: entry for entry in entries}}, indent=2)
    )
