"""Compiler fixtures use the existing bounded launcher, including its wait.

Literal command routing is source-checkable; dynamic fixture helper routing
and contained environment preservation have subprocess tests in the gate owner.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest
from capsem_builder.gate import boundedlease, config

ROOT = Path(__file__).resolve().parents[2]
SETTINGS = config.load(ROOT).locks.bounded
LEASE_RATIONALE = (
    "AGENTS.md 'Bound Direct Diagnostics': Cargo fixtures share the machine lease. "
    "The bounded wrapper owns a finite child timeout after its lock wait; an "
    "outer subprocess timeout can abandon the owning launcher while it queues."
)


def _violations(source: str) -> list[int]:
    violations = []
    for node in ast.walk(ast.parse(source)):
        if not (
            isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
            and isinstance(node.func.value, ast.Name) and node.func.value.id == "subprocess"
            and node.func.attr in {"run", "Popen", "check_output", "check_call"} and node.args
        ):
            continue
        argv = node.args[0]
        if isinstance(argv, (ast.List, ast.Tuple)):
            tokens = [item.value if isinstance(item, ast.Constant) and isinstance(item.value, str)
                      else "_dynamic_argument_" for item in argv.elts]
            if boundedlease.machine_work(tokens, SETTINGS):
                violations.append(node.lineno)
        if (
            isinstance(argv, ast.Call) and isinstance(argv.func, ast.Name) and argv.func.id == "bounded"
            and any(keyword.arg == "timeout" for keyword in node.keywords)
        ):
            violations.append(node.lineno)
    return violations


def test_literal_fixture_machine_work_has_one_bounded_owner() -> None:
    offenders = [
        f"{path.relative_to(ROOT)}:{line}"
        for base in (ROOT / "tests", ROOT / "build_system/tests")
        for path in sorted(base.rglob("*.py"))
        for line in _violations(path.read_text(encoding="utf-8"))
    ]
    assert not offenders, LEASE_RATIONALE + "\n" + "\n".join(offenders)


@pytest.mark.parametrize("source", [
    'subprocess.run(["cargo", "build"])',
    'subprocess.Popen(("cargo", "+1.97.1", "nextest", "list"))',
    'subprocess.run(["env", "RUSTC_WRAPPER=", "cargo", "clean"])',
    'subprocess.run(bounded(["cargo", "test"], 120), timeout=120)',
])
def test_a_fixture_cannot_bypass_or_short_circuit_the_lease(source: str) -> None:
    assert _violations(source), LEASE_RATIONALE


@pytest.mark.parametrize("source", [
    'subprocess.run(bounded(["cargo", "build"], 120))',
    'subprocess.run(["cargo", "metadata", "--no-deps"])',
    'subprocess.run(["cargo", "--version"])',
    'subprocess.run(["python", "-c", "cargo build"])',
])
def test_read_only_commands_and_the_bounded_owner_are_preserved(source: str) -> None:
    assert not _violations(source), LEASE_RATIONALE
