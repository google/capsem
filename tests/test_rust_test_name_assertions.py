"""Source contracts must look for Rust test names where the tests actually are.

`CLAUDE.md` requires every `#[test]` to live in a sibling `tests.rs`. A Python
source contract that asserts a Rust test exists must therefore read that
sibling, not the production file the test was moved out of -- see
`tests/rust_sources.py`.

This has already cost two release attempts. Sixteen contracts under
`tests/capsem-release/` searched production files for relocated test names, and
five more under `tests/capsem-install/` did the same; the second set runs only
inside the Docker install gate, so it stayed invisible until forty minutes into
a release run. Nothing but this test connects a Rust file layout change to the
Python assertions that depend on it, and both failures looked like a broken
release rather than a moved function.
"""

from __future__ import annotations

import ast
import re
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent
CRATES = PROJECT_ROOT / "crates"
PYTHON_TESTS = PROJECT_ROOT / "tests"

_FN = re.compile(r"(?m)^\s*(?:pub\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)")
# Long enough to be a distinctive test name rather than an incidental word.
_LITERAL = re.compile(r'"([a-z_][a-z0-9_]{15,})"')


def _is_test_module(path: Path) -> bool:
    return path.name == "tests.rs" or path.name.endswith("_tests.rs")


def _names_only_defined_in_test_modules() -> set[str]:
    in_tests: set[str] = set()
    in_production: set[str] = set()
    for rs in CRATES.rglob("*.rs"):
        names = set(_FN.findall(rs.read_text(encoding="utf-8", errors="ignore")))
        (in_tests if _is_test_module(rs) else in_production).update(names)
    return in_tests - in_production


def _production_rust_readers(tree: ast.AST, source: str) -> set[str]:
    """Variables in `tree` assigned from reading a production Rust file.

    Resolved from the assignment rather than from the mere presence of a name
    somewhere in the file: several contracts legitimately mention a relocated
    test name while asserting it against a spec document or a Rust test file,
    and flagging those would make this guard a source of exemptions instead of
    a source of truth.
    """
    readers: set[str] = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            continue
        target = node.targets[0]
        if not isinstance(target, ast.Name):
            continue
        expr = ast.get_source_segment(source, node.value) or ""
        if "read_text" not in expr and "production(" not in expr:
            continue
        # A path spelled in the assignment, or a module constant holding one.
        paths = re.findall(r'"([^"]*\.rs)"', expr) + [
            ref
            for ref in re.findall(r"\b([A-Z][A-Z0-9_]{2,})\b", expr)
            if ref.endswith("_RS") or "RS" in ref or "GRAPH" in ref or "ADMIN" in ref
        ]
        if not paths:
            continue
        if any(path.endswith(("tests.rs", "_tests.rs")) for path in paths):
            continue
        readers.add(target.id)
    return readers


def test_no_contract_seeks_a_test_name_in_production_source() -> None:
    relocated = _names_only_defined_in_test_modules()
    assert relocated, "no sibling test modules found; this guard would pass vacuously"

    offenders: dict[str, set[str]] = {}
    for py in PYTHON_TESTS.rglob("*.py"):
        source = py.read_text(encoding="utf-8", errors="ignore")
        try:
            tree = ast.parse(source)
        except SyntaxError:
            continue
        named: set[str] = set()
        # Resolved per function: variable names are function-scoped, and the
        # same name reading a test module in one test and production source in
        # another must not taint both.
        for scope in ast.walk(tree):
            if not isinstance(scope, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            readers = _production_rust_readers(scope, source)
            if not readers:
                continue
            for node in ast.walk(scope):
                if not isinstance(node, ast.Compare) or len(node.ops) != 1:
                    continue
                if not isinstance(node.ops[0], ast.In):
                    continue
                left, right = node.left, node.comparators[0]
                if not (isinstance(left, ast.Constant) and isinstance(left.value, str)):
                    continue
                if isinstance(right, ast.Name) and right.id in readers:
                    if left.value in relocated:
                        named.add(left.value)
        if named:
            offenders[str(py.relative_to(PROJECT_ROOT))] = named

    assert not offenders, (
        "these contracts name Rust tests that live only in a sibling test "
        "module, but do not read it through rust_sources.sibling_tests -- they "
        "are searching production source for a function that is not there:\n"
        + "\n".join(
            f"  {path}: {', '.join(sorted(names))}"
            for path, names in sorted(offenders.items())
        )
    )
