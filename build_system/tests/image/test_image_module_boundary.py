"""Guard the image builder's direct package and command ownership."""

from __future__ import annotations

import importlib
import tomllib
from pathlib import Path

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
IMAGE_ROOT = BUILD_SYSTEM_ROOT / "builder" / "image"

EXPECTED_MODULES = {
    "__init__.py",
    "assetdependencies.py",
    "assettools.py",
    "audit.py",
    "cli.py",
    "config.py",
    "docker.py",
    "doctor.py",
    "guestbuilder.py",
    "image_build_backend.py",
    "manifest.py",
    "models.py",
    "schema.py",
    "skills.py",
    "validate.py",
}


def test_image_builder_has_one_exact_source_owner() -> None:
    assert {path.name for path in IMAGE_ROOT.glob("*.py")} == EXPECTED_MODULES
    assert not (REPOSITORY_ROOT / "src" / "capsem" / "builder").exists()


def test_builder_distribution_owns_image_package_and_command() -> None:
    project = tomllib.loads(
        (BUILD_SYSTEM_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    )
    assert "capsem_builder.image" in project["tool"]["setuptools"]["packages"]
    assert project["project"]["scripts"]["capsem-builder"] == (
        "capsem_builder.image.cli:main"
    )


def test_image_modules_resolve_from_the_direct_source_tree() -> None:
    for name in ("config", "models", "schema", "cli", "doctor"):
        module = importlib.import_module(f"capsem_builder.image.{name}")
        assert module.__file__ is not None
        assert Path(module.__file__).resolve() == (IMAGE_ROOT / f"{name}.py").resolve()
