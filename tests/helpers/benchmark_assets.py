"""Isolated runtime assets for host-side route benchmarks."""

from __future__ import annotations

import json
import shutil
from pathlib import Path


def clone_benchmark_assets(source: Path, destination: Path) -> Path:
    """Clone an asset tree cheaply and install deterministic status metadata."""

    def link_file(path: str, target: str) -> str:
        Path(target).symlink_to(Path(path).resolve())
        return target

    shutil.copytree(source, destination, copy_function=link_file, symlinks=True)
    metadata = destination / "manifest-metadata.json"
    metadata.unlink(missing_ok=True)
    metadata.write_text(
        json.dumps(
            {
                "schema": "capsem.manifest_metadata.v1",
                "origin": "benchmark_fixture",
                "checked_at": 1_800_000_000,
                "update_available": False,
                "validation_status": "valid",
            }
        ),
        encoding="utf-8",
    )
    return destination
