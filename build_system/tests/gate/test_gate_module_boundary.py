"""Guard the gate's direct package, launcher, and command ownership."""

from __future__ import annotations

import importlib
import tomllib
from pathlib import Path

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
BUILDER_ROOT = BUILD_SYSTEM_ROOT / "builder"
GATE_ROOT = BUILDER_ROOT / "gate"


def test_gate_has_one_direct_source_owner() -> None:
    assert (GATE_ROOT / "__init__.py").is_file()
    assert not (REPOSITORY_ROOT / "src" / "capsem" / "gate").exists()
    assert (BUILDER_ROOT / "gatelaunch.py").is_file()
    assert not (REPOSITORY_ROOT / "src" / "capsem" / "gatelaunch.py").exists()


def test_builder_distribution_owns_gate_package_and_command() -> None:
    project = tomllib.loads(
        (BUILD_SYSTEM_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    )
    assert "capsem_builder.gate" in project["tool"]["setuptools"]["packages"]
    assert project["project"]["scripts"]["capsem-gate"] == (
        "capsem_builder.gatelaunch:main"
    )


def test_gate_modules_resolve_from_the_direct_source_tree() -> None:
    for name in (
        "actions",
        "cli",
        "execution",
        "plan",
        "qualification",
        "runlog",
        "sourcecommit",
    ):
        module = importlib.import_module(f"capsem_builder.gate.{name}")
        assert module.__file__ is not None
        assert Path(module.__file__).resolve() == (GATE_ROOT / f"{name}.py").resolve()

    launcher = importlib.import_module("capsem_builder.gatelaunch")
    assert launcher.__file__ is not None
    assert Path(launcher.__file__).resolve() == (BUILDER_ROOT / "gatelaunch.py").resolve()
