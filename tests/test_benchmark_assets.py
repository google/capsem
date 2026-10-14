"""The route benchmark owns stable installed-status inputs."""

from __future__ import annotations

import json
from pathlib import Path

from helpers.benchmark_assets import clone_benchmark_assets


def test_clone_benchmark_assets_is_isolated_and_deterministic(tmp_path: Path) -> None:
    source = tmp_path / "source"
    source.mkdir()
    (source / "manifest.json").write_text('{"format": 2}')
    original = '{"schema": "ambient"}'
    (source / "manifest-metadata.json").write_text(original)
    (source / "large-asset").write_bytes(b"asset")

    cloned = clone_benchmark_assets(source, tmp_path / "cloned")

    assert (cloned / "manifest.json").is_symlink()
    assert (cloned / "large-asset").is_symlink()
    assert not (cloned / "manifest-metadata.json").is_symlink()
    assert json.loads((cloned / "manifest-metadata.json").read_text()) == {
        "schema": "capsem.manifest_metadata.v1",
        "origin": "benchmark_fixture",
        "checked_at": 1_800_000_000,
        "update_available": False,
        "validation_status": "valid",
    }
    assert (source / "manifest-metadata.json").read_text() == original
