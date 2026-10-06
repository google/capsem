"""Compiler fixtures use the existing bounded launcher, including its wait.

Literal command routing is source-checkable; dynamic fixture helper routing
and contained environment preservation have subprocess tests in the gate owner.
"""

from __future__ import annotations

import ast
import shlex
from pathlib import Path

import pytest
from capsem_builder.gate import boundedlease, config
from pydantic import ValidationError

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


def test_install_asset_script_queues_before_its_outer_timeout_starts() -> None:
    command = ["bash", "build_system/scripts/test/prepare-install-test-assets.sh"]
    assert boundedlease.machine_work(command, SETTINGS), LEASE_RATIONALE
    assert boundedlease.machine_work(["env", "CAPSEM_ARCH=x86_64", *command], SETTINGS)
    assert not boundedlease.machine_work(["bash", "-c", "echo cargo build"], SETTINGS)
    assert not boundedlease.machine_work(["bash", "unrelated.sh"], SETTINGS)


def test_install_asset_script_leases_direct_compilation() -> None:
    source = (ROOT / "build_system/scripts/test/prepare-install-test-assets.sh").read_text()
    commands = [shlex.split(line) for line in source.replace("\\\n", " ").splitlines()
                if "cargo run -p capsem-admin" in line]
    assert len(commands) == 1
    command = commands[0]
    assert command[:3] == ["python3", "build_system/scripts/ci/run-bounded-command.py", "--timeout-seconds"]
    assert int(command[3]) > 0
    assert command[4:8] == ["--", "cargo", "run", "-p"], LEASE_RATIONALE


@pytest.mark.parametrize("prefix", [[], [""], ["bash", ""]])
def test_machine_work_prefix_cannot_match_every_command(prefix: list[str]) -> None:
    with pytest.raises(ValidationError, match="nonempty tokens"):
        type(SETTINGS).model_validate({**SETTINGS.model_dump(), "command_prefixes": [prefix]})


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
