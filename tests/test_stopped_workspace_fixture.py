"""The stopped-workspace fixture seeds only an isolated registry."""

from __future__ import annotations

import stat
from pathlib import Path

import pytest
from helpers import stopped_workspace
from helpers.stopped_workspace import seed_stopped_workspace


def test_fixture_never_replaces_an_existing_registry(tmp_path: Path) -> None:
    registry = tmp_path / "persistent_registry.json"
    registry.write_text("existing registry")
    with pytest.raises(ValueError, match="isolated empty registry"):
        seed_stopped_workspace(tmp_path, tmp_path / "assets")
    assert registry.read_text() == "existing registry"
    assert not (tmp_path / "persistent").exists()


def test_fixture_makes_ledger_files_private(tmp_path: Path, monkeypatch) -> None:
    real_copy2 = stopped_workspace.shutil.copy2
    real_copytree = stopped_workspace.shutil.copytree

    def permissive_copy2(source: Path, destination: Path) -> Path:
        copied = Path(real_copy2(source, destination))
        copied.chmod(0o644)
        return copied

    def permissive_copytree(source: Path, destination: Path) -> Path:
        copied = real_copytree(source, destination)
        copied.chmod(0o755)
        return copied

    monkeypatch.setattr(stopped_workspace, "manifest_asset_pins", lambda _assets: {})
    monkeypatch.setattr(stopped_workspace.shutil, "copy2", permissive_copy2)
    monkeypatch.setattr(stopped_workspace.shutil, "copytree", permissive_copytree)

    seed_stopped_workspace(tmp_path, tmp_path / "assets")

    mode = stat.S_IMODE(
        (tmp_path / "persistent" / stopped_workspace.VM_ID / "session.bodies")
        .stat()
        .st_mode
    )
    assert mode == 0o700
    session = tmp_path / "persistent" / stopped_workspace.VM_ID
    assert stat.S_IMODE((session / "session.db").stat().st_mode) == 0o600
    assert stat.S_IMODE((session / "session.db-archive.lock").stat().st_mode) == 0o600
