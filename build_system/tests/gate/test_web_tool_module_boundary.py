"""Guard direct package ownership for the remaining web command tools."""

from __future__ import annotations

import ast
import importlib
import stat
import tomllib
from pathlib import Path

from pytest import MonkeyPatch

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
TOOL_ROOT = BUILD_SYSTEM_ROOT / "builder" / "gate" / "tools" / "web"

COMMANDS = {
    "check-cloudflare-pages-project.py": "check_cloudflare_pages_project",
    "check-docs-holding-build.py": "check_docs_holding_build",
    "cloudflare_pages_rollback.py": "cloudflare_pages_rollback",
}


def test_web_tools_close_the_exact_owned_package() -> None:
    assert {path.stem for path in TOOL_ROOT.glob("*.py")} == {
        "__init__",
        *COMMANDS.values(),
    }


def test_web_tools_ship_in_the_builder_distribution() -> None:
    project_file = BUILD_SYSTEM_ROOT / ("pyproject" + ".toml")
    project = tomllib.loads(project_file.read_text(encoding="utf-8"))
    packages = project["tool"]["setuptools"]["packages"]
    assert packages.count("capsem_builder.gate.tools.web") == 1


def test_web_tools_import_without_a_repository_checkout(
    monkeypatch: MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("CAPSEM_REPOSITORY_ROOT", raising=False)
    for module in COMMANDS.values():
        imported = importlib.import_module(f"capsem_builder.gate.tools.web.{module}")
        importlib.reload(imported)


def test_web_launchers_are_thin_direct_commands() -> None:
    for name, module in COMMANDS.items():
        path = REPOSITORY_ROOT / "build_system" / "scripts" / "web" / name
        source = path.read_text(encoding="utf-8")
        tree = ast.parse(source)
        assert len(source.splitlines()) <= 21, f"{name} contains reusable behavior"
        imports = [
            node
            for node in tree.body
            if isinstance(node, ast.ImportFrom)
            and node.module == f"capsem_builder.gate.tools.web.{module}"
        ]
        assert len(imports) == 1, f"{name} does not import its direct owner"
        assert [alias.name for alias in imports[0].names] == ["main"]
        assert not any(
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))
            for node in tree.body
        )
        exits = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Raise)
            and isinstance(node.exc, ast.Call)
            and isinstance(node.exc.func, ast.Name)
            and node.exc.func.id == "SystemExit"
        ]
        assert len(exits) == 1, f"{name} does not propagate command status"
        assert not bool(path.stat().st_mode & stat.S_IXUSR), f"{name} changed mode"


def test_web_tools_do_not_import_through_compatibility_launchers() -> None:
    for module in COMMANDS.values():
        path = TOOL_ROOT / f"{module}.py"
        tree = ast.parse(path.read_text(encoding="utf-8"))
        for node in ast.walk(tree):
            if isinstance(node, ast.ImportFrom) and node.module is not None:
                assert not node.module.startswith("scripts.")
