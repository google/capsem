"""Guard the gate's direct package, launcher, and command ownership."""

from __future__ import annotations

import ast
import importlib
import tomllib
from pathlib import Path

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
BUILDER_ROOT = BUILD_SYSTEM_ROOT / "builder"
GATE_ROOT = BUILDER_ROOT / "gate"
AUDIT_SCRIPT_ROOT = BUILD_SYSTEM_ROOT / "scripts" / "audit"


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
    assert "capsem_builder.gate.tools" in project["tool"]["setuptools"]["packages"]
    assert "capsem_builder.gate.tools.audit" in project["tool"]["setuptools"]["packages"]
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


def test_audit_script_boundaries_are_thin_exit_status_launchers() -> None:
    launchers = {
        "audit-pnpm-bulk.py": "pnpm_bulk",
        "check-cargo-audit.py": "cargo_audit",
        "check-dependency-drift.py": "dependency_drift",
        "check-hardcoded-release-selections.py": "release_selections",
        "check-source-syntax.py": "source_syntax",
        "check-surfaces.py": "surfaces",
        "check_public_surface.py": "public_surface",
    }
    for name, module in launchers.items():
        source = (AUDIT_SCRIPT_ROOT / name).read_text(encoding="utf-8")
        tree = ast.parse(source)
        assert len(source.splitlines()) <= 20, f"{name} contains reusable behavior"
        assert f"capsem_builder.gate.tools.audit.{module}" in source
        assert not any(isinstance(node, (ast.FunctionDef, ast.ClassDef)) for node in tree.body)
        exits = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Raise)
            and isinstance(node.exc, ast.Call)
            and isinstance(node.exc.func, ast.Name)
            and node.exc.func.id == "SystemExit"
        ]
        assert len(exits) == 1, f"{name} does not propagate its owned command status"

    harness = (AUDIT_SCRIPT_ROOT / "lint_harness.py").read_text(encoding="utf-8")
    assert len(harness.splitlines()) <= 25
    assert "capsem_builder.gate.tools.audit.lint_harness" in harness


def test_audit_configuration_and_boundaries_have_one_functional_owner() -> None:
    expected = {
        "audit-pnpm-bulk.py",
        "audit-python-lock.sh",
        "audit.toml",
        "check-cargo-audit.py",
        "check-dependency-drift.py",
        "check-hardcoded-release-selections.py",
        "check-hardcoded-release-selections.sh",
        "check-source-syntax.py",
        "check-surfaces.py",
        "check_public_surface.py",
        "lint_harness.py",
    }
    assert {path.name for path in AUDIT_SCRIPT_ROOT.iterdir()} == expected
    assert not (REPOSITORY_ROOT / "audit.toml").exists()
    for name in expected - {"audit.toml"}:
        assert not (REPOSITORY_ROOT / "scripts" / name).exists()
