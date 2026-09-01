"""Affected-Rust selection follows Cargo ownership and reverse dependencies."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "build_system/scripts/test/rust-affected.py"
SPEC = importlib.util.spec_from_file_location("rust_affected", SCRIPT)
assert SPEC and SPEC.loader
rust_affected = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = rust_affected
SPEC.loader.exec_module(rust_affected)


def _package(name: str, dependencies: set[str] | None = None):
    return rust_affected.Package(
        name=name,
        root=PurePosixPath("crates") / name,
        dependencies=frozenset(dependencies or ()),
    )


def _workspace():
    return {
        "capsem-foundation": _package("capsem-foundation"),
        "capsem-core": _package("capsem-core", {"capsem-foundation"}),
        "capsem-service": _package("capsem-service", {"capsem-core"}),
        "capsem": _package("capsem", {"capsem-core"}),
        "capsem-tray": _package("capsem-tray", {"capsem-foundation"}),
    }


def test_dependency_parser_covers_target_build_dev_and_renamed_dependencies() -> None:
    manifest = {
        "dependencies": {"core": {"package": "capsem-core", "path": "../core"}},
        "build-dependencies": {"capsem-proto": "1"},
        "dev-dependencies": {"capsem-mock-server": {"path": "../mock"}},
        "target": {"cfg(unix)": {"dependencies": {"capsem-foundation": "1"}}},
    }
    assert rust_affected.dependency_names(manifest) == {
        "capsem-core",
        "capsem-foundation",
        "capsem-mock-server",
        "capsem-proto",
    }


def test_changed_owner_selects_every_transitive_reverse_dependent() -> None:
    selected = rust_affected.affected_packages(
        _workspace(),
        (PurePosixPath("crates/capsem-foundation/src/paths.rs"),),
    )
    assert selected == {
        "capsem-foundation",
        "capsem-core",
        "capsem-service",
        "capsem",
        "capsem-tray",
    }


def test_leaf_change_stays_focused() -> None:
    selected = rust_affected.affected_packages(
        _workspace(),
        (PurePosixPath("crates/capsem-service/src/api.rs"),),
    )
    assert selected == {"capsem-service"}


def test_clean_tree_and_root_contract_changes_select_the_workspace() -> None:
    packages = _workspace()
    assert rust_affected.affected_packages(packages, ()) == frozenset(packages)
    for path in ("Cargo.toml", "Cargo.lock", "rustfmt.toml", ".cargo/config.toml"):
        assert rust_affected.affected_packages(packages, (PurePosixPath(path),)) == frozenset(packages)


def test_non_rust_changes_are_an_explicit_noop() -> None:
    selected = rust_affected.affected_packages(
        _workspace(),
        (PurePosixPath("web/app/readme.md"),),
    )
    assert selected == frozenset()
    assert rust_affected.cargo_test_command(_workspace(), selected) is None


def test_command_is_workspace_wide_or_exactly_package_scoped() -> None:
    packages = _workspace()
    assert rust_affected.cargo_test_command(packages, frozenset(packages)) == (
        "cargo",
        "test",
        "--all-targets",
        "--all-features",
        "--workspace",
    )
    assert rust_affected.cargo_test_command(packages, frozenset({"capsem", "capsem-core"})) == (
        "cargo",
        "test",
        "--all-targets",
        "--all-features",
        "-p",
        "capsem",
        "-p",
        "capsem-core",
    )


def test_real_workspace_inventory_matches_cargo_members() -> None:
    packages = rust_affected.workspace_packages(ROOT)
    assert set(packages) == {
        "capsem",
        "capsem-admin",
        "capsem-agent",
        "capsem-app",
        "capsem-assets",
        "capsem-bench",
        "capsem-config",
        "capsem-core",
        "capsem-credentials",
        "capsem-foundation",
        "capsem-gateway",
        "capsem-guard",
        "capsem-logger",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-mock-server",
        "capsem-process",
        "capsem-proto",
        "capsem-service",
        "capsem-tray",
        "capsem-tui",
    }
    assert "capsem-core" in packages["capsem-service"].dependencies
    assert "capsem-service" not in packages["capsem-core"].dependencies
