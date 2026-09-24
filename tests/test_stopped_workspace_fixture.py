"""The stopped-workspace fixture seeds only an isolated registry."""

from __future__ import annotations

from pathlib import Path

import pytest
from helpers.stopped_workspace import seed_stopped_workspace


def test_fixture_never_replaces_an_existing_registry(tmp_path: Path) -> None:
    registry = tmp_path / "persistent_registry.json"
    registry.write_text("existing registry")
    with pytest.raises(ValueError, match="isolated empty registry"):
        seed_stopped_workspace(tmp_path, tmp_path / "profiles")
    assert registry.read_text() == "existing registry"
    assert not (tmp_path / "persistent").exists()
