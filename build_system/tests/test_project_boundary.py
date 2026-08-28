from __future__ import annotations

import importlib
from importlib.metadata import version
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def test_editable_install_maps_the_direct_source_directory() -> None:
    capsem_builder = importlib.import_module("capsem_builder")
    assert capsem_builder.__name__ == "capsem_builder"
    assert capsem_builder.__file__ is not None
    assert Path(capsem_builder.__file__).resolve() == (
        PROJECT_ROOT / "builder" / "__init__.py"
    ).resolve()


def test_installed_distribution_uses_the_reserved_builder_name() -> None:
    assert version("capsem-builder") == "0.6.2"
