"""Keep dynamically loaded script consumers tied to the script's real API.

Python's type checker sees modules created by ``spec_from_file_location`` as
``ModuleType``.  A test can therefore keep calling a helper after that helper
moves to another script, while Ruff and ty both stay green.  A release did
exactly that and spent a hosted dispatch discovering the stale call.

This guard resolves the release-critical loaders whose paths are static, maps
local variables assigned from those loaders, and checks their attribute access
against names the target script actually defines or imports.
"""

from __future__ import annotations

import ast
from collections.abc import Iterable
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_TEST_ROOTS = (
    PROJECT_ROOT / "tests" / "capsem-release",
    PROJECT_ROOT / "tests" / "capsem-build-chain",
)

RATIONALE = """\
A release-critical test calls an attribute missing from its dynamically loaded
script. Dynamic ModuleType loaders bypass Python's type checker, so stale calls
otherwise survive fast source checks and fail only in the hosted release
contracts. Update the consumer to load the module that owns the attribute, or
restore an intentional public API on the loaded script.
"""


def _call_name(node: ast.AST) -> str | None:
    if not isinstance(node, ast.Call):
        return None
    function = node.func
    if isinstance(function, ast.Name):
        return function.id
    if isinstance(function, ast.Attribute):
        return function.attr
    return None


def _target_names(target: ast.AST) -> Iterable[str]:
    if isinstance(target, ast.Name):
        yield target.id
    elif isinstance(target, (ast.List, ast.Tuple)):
        for child in target.elts:
            yield from _target_names(child)


def _assignment(node: ast.AST) -> tuple[list[ast.expr], ast.expr] | None:
    if isinstance(node, ast.Assign):
        return node.targets, node.value
    if isinstance(node, ast.AnnAssign) and node.value is not None:
        return [node.target], node.value
    return None


def _resolve_path(node: ast.AST, known: dict[str, Path]) -> Path | None:
    if isinstance(node, ast.Name):
        return known.get(node.id)
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return Path(node.value)
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Div):
        left = _resolve_path(node.left, known)
        right = _resolve_path(node.right, known)
        if left is not None and right is not None:
            return left / right
    return None


def _path_bindings(nodes: Iterable[ast.AST], initial: dict[str, Path]) -> dict[str, Path]:
    known = dict(initial)
    assignments = [assignment for node in nodes if (assignment := _assignment(node))]
    for _ in range(len(assignments) + 1):
        changed = False
        for targets, value in assignments:
            resolved = _resolve_path(value, known)
            if resolved is None:
                continue
            for target in targets:
                for name in _target_names(target):
                    if known.get(name) != resolved:
                        known[name] = resolved
                        changed = True
        if not changed:
            break
    return known


def _loader_targets(tree: ast.Module) -> dict[str, Path]:
    roots = {"PROJECT_ROOT": PROJECT_ROOT, "ROOT": PROJECT_ROOT}
    globals_ = _path_bindings(tree.body, roots)
    loaders: dict[str, Path] = {}
    for function in tree.body:
        if not isinstance(function, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        paths = _path_bindings(ast.walk(function), globals_)
        for node in ast.walk(function):
            if _call_name(node) != "spec_from_file_location" or not isinstance(node, ast.Call):
                continue
            if len(node.args) < 2:
                continue
            target = _resolve_path(node.args[1], paths)
            if target is not None and target.is_file() and target.is_relative_to(PROJECT_ROOT):
                loaders[function.name] = target
                break
    return loaders


def _defined_names(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    names: set[str] = set()

    def collect(statements: Iterable[ast.stmt]) -> None:
        for statement in statements:
            if isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names.add(statement.name)
            elif isinstance(statement, (ast.Assign, ast.AnnAssign)):
                assignment = _assignment(statement)
                assert assignment is not None
                for target in assignment[0]:
                    names.update(_target_names(target))
            elif isinstance(statement, ast.Import):
                names.update(alias.asname or alias.name.split(".")[0] for alias in statement.names)
            elif isinstance(statement, ast.ImportFrom):
                names.update(alias.asname or alias.name for alias in statement.names)
            elif isinstance(statement, ast.If):
                collect(statement.body)
                collect(statement.orelse)
            elif isinstance(statement, ast.Try):
                collect(statement.body)
                for handler in statement.handlers:
                    collect(handler.body)
                collect(statement.orelse)
                collect(statement.finalbody)

    collect(tree.body)
    return names


def _missing_dynamic_attributes(path: Path) -> list[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    loaders = _loader_targets(tree)
    missing: list[str] = []
    for function in ast.walk(tree):
        if not isinstance(function, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        modules: dict[str, Path] = {}
        for node in ast.walk(function):
            assignment = _assignment(node)
            if assignment is None:
                continue
            targets, value = assignment
            loader = _call_name(value)
            if loader not in loaders:
                continue
            for target in targets:
                for name in _target_names(target):
                    modules[name] = loaders[loader]
        for node in ast.walk(function):
            if not isinstance(node, ast.Attribute) or not isinstance(node.value, ast.Name):
                continue
            target = modules.get(node.value.id)
            if target is None or node.attr in _defined_names(target):
                continue
            missing.append(
                f"{path.relative_to(PROJECT_ROOT)}:{node.lineno}: "
                f"{node.value.id}.{node.attr} is not defined by "
                f"{target.relative_to(PROJECT_ROOT)}"
            )
    return missing


def test_dynamic_release_script_consumers_use_real_module_apis() -> None:
    tests = sorted(path for root in RELEASE_TEST_ROOTS for path in root.rglob("test_*.py"))
    missing = [detail for path in tests for detail in _missing_dynamic_attributes(path)]
    assert not missing, RATIONALE + "\n\n" + "\n".join(missing)
