"""Guard the image builder's direct package and command ownership."""

from __future__ import annotations

import ast
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
    assert "capsem_builder.image.tools" in project["tool"]["setuptools"]["packages"]
    assert "capsem_builder.image.tools.bootstrap" in project["tool"]["setuptools"]["packages"]
    assert "capsem_builder.image.tools.build" in project["tool"]["setuptools"]["packages"]
    assert project["project"]["scripts"]["capsem-builder"] == (
        "capsem_builder.image.cli:main"
    )


def test_image_modules_resolve_from_the_direct_source_tree() -> None:
    for name in ("config", "models", "schema", "cli", "doctor"):
        module = importlib.import_module(f"capsem_builder.image.{name}")
        assert module.__file__ is not None
        assert Path(module.__file__).resolve() == (IMAGE_ROOT / f"{name}.py").resolve()


def test_bootstrap_script_boundaries_are_thin_exit_status_launchers() -> None:
    launchers = {
        "install-configured-cargo-tools.py": "cargo_tools",
        "prepare-linux-sandbox.py": "linux_sandbox",
        "provision-linux-workspace.py": "linux_workspace",
    }
    for name, module in launchers.items():
        source = (
            REPOSITORY_ROOT / "build_system" / "scripts" / "bootstrap" / name
        ).read_text(encoding="utf-8")
        tree = ast.parse(source)
        assert len(source.splitlines()) <= 20, f"{name} contains reusable behavior"
        assert f"capsem_builder.image.tools.bootstrap.{module}" in source
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


def test_build_script_boundaries_are_thin_image_owned_launchers() -> None:
    launchers = {
        "archive_db_writer_benchmark.py": "archive_db_writer_benchmark",
        "benchmark_report.py": "benchmark_report",
        "check-macos-native-glowup.py": "check_macos_native_glowup",
        "clean_stale.py": "clean_stale",
        "create_hash_assets.py": "create_hash_assets",
        "docker-storage-policy.py": "docker_storage_policy",
        "gen_manifest.py": "gen_manifest",
        "materialize-package-ort.py": "materialize_package_ort",
        "print-gate-digest.py": "print_gate_digest",
        "prune-benchmark-history.py": "prune_benchmark_history",
        "resolve-reusable-profile-assets.py": "resolve_reusable_profile_assets",
        "run-installed-winterfell.py": "run_installed_winterfell",
        "stage_profile_assets.py": "stage_profile_assets",
        "sync-container-clock.py": "sync_container_clock",
        "tart_readiness.py": "tart_readiness",
    }
    for name, module in launchers.items():
        source = (REPOSITORY_ROOT / "scripts" / name).read_text(encoding="utf-8")
        tree = ast.parse(source)
        assert len(source.splitlines()) <= 21, f"{name} contains reusable behavior"
        imports = {
            alias.name
            for node in tree.body
            if isinstance(node, ast.ImportFrom)
            and node.module == "capsem_builder.image.tools.build"
            for alias in node.names
        }
        assert module in imports
        assert not any(isinstance(node, (ast.FunctionDef, ast.ClassDef)) for node in tree.body)
