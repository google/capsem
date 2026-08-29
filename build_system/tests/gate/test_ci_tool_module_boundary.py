"""Guard direct package ownership for reusable CI/process tools."""

from __future__ import annotations

import ast
import tomllib
from pathlib import Path

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
CI_TOOL_ROOT = BUILD_SYSTEM_ROOT / "builder" / "gate" / "tools" / "ci"
CI_SCRIPT_ROOT = BUILD_SYSTEM_ROOT / "scripts" / "ci"

LAUNCHERS = {
    "check-orphan-processes.py": "check_orphan_processes",
    "classify-ci-scope.py": "classify_ci_scope",
    "gate-tool-list.py": "gate_tool_list",
    "require-clean-worktree.py": "require_clean_worktree",
    "run-bounded-command.py": "run_bounded_command",
}
IMPORT_ADAPTERS = {"justfile-graph.py": "justfile_graph"}


def test_ci_tools_have_one_exact_gate_owned_package() -> None:
    assert {path.stem for path in CI_TOOL_ROOT.glob("*.py")} == {
        "__init__",
        *LAUNCHERS.values(),
        *IMPORT_ADAPTERS.values(),
    }
    project = tomllib.loads(
        (BUILD_SYSTEM_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    )
    assert "capsem_builder.gate.tools.ci" in project["tool"]["setuptools"]["packages"]


def test_ci_boundaries_have_one_exact_functional_owner() -> None:
    names = {*LAUNCHERS, *IMPORT_ADAPTERS, "require-ci-jobs.sh"}
    assert {path.name for path in CI_SCRIPT_ROOT.iterdir()} == names
    assert not any((REPOSITORY_ROOT / "scripts" / name).exists() for name in names)


def test_ci_script_boundaries_are_thin_gate_owned_launchers() -> None:
    for name, module in LAUNCHERS.items():
        source = (CI_SCRIPT_ROOT / name).read_text(encoding="utf-8")
        tree = ast.parse(source)
        assert len(source.splitlines()) <= 21, f"{name} contains reusable behavior"
        imports = {
            alias.name
            for node in tree.body
            if isinstance(node, ast.ImportFrom)
            and node.module == "capsem_builder.gate.tools.ci"
            for alias in node.names
        }
        assert module in imports
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


def test_justfile_graph_is_a_thin_compatibility_import_adapter() -> None:
    name, module = next(iter(IMPORT_ADAPTERS.items()))
    source = (CI_SCRIPT_ROOT / name).read_text(encoding="utf-8")
    tree = ast.parse(source)
    assert len(source.splitlines()) <= 20, f"{name} contains reusable behavior"
    assert f"capsem_builder.gate.tools.ci.{module}" in source
    assert not any(isinstance(node, (ast.FunctionDef, ast.ClassDef)) for node in tree.body)
