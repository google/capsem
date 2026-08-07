"""No plan action starts a second gate, and every reference it names exists.

`GuardedRunner` refuses re-entry at runtime, which is the enforcement that
matters -- it cannot be forgotten and it cannot be argued with. This is the
second layer, and it earns its place by answering a different question: the
runtime guard tells you at minute thirty of a held lock, this one tells you at
the commit that introduced it, with the file and line.

The cross-reference half exists because of `_build-host-image`. Two modules
invoked that recipe, the recipe had a heading in the justfile and no body, and
`just --show _build-host-image` had been failing for as long as the code had
been there. Nothing noticed, because nothing checked that a name written in
Python resolves to something real.
"""

from __future__ import annotations

import ast
import json
import subprocess
from pathlib import Path

import pytest

from capsem.gate import cli  # noqa: F401 - imported so every command registers
from capsem.gate import config as gate_config
from capsem.gate.command import GateCommand
from capsem.gate.funnel import ENTRYPOINTS, program

PROJECT_ROOT = Path(__file__).resolve().parents[1]
GATE_PACKAGE = PROJECT_ROOT / "src" / "capsem" / "gate"

CONFIG = gate_config.load(PROJECT_ROOT)


def _recipes() -> dict[str, str]:
    """Every just recipe, with the body text a dispatch can be read out of."""
    dump = subprocess.run(
        ["just", "--dump", "--dump-format", "json"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    recipes = json.loads(dump.stdout)["recipes"]
    return {
        name: " ".join(
            "".join(part for part in line if isinstance(part, str))
            for line in body["body"]
        )
        for name, body in recipes.items()
    }


RECIPES = _recipes()


def _subcommand_of(recipe: str) -> str | None:
    """The gate subcommand a recipe dispatches to, if it dispatches."""
    body = RECIPES.get(recipe)
    if body is None:
        return None
    parts = body.split()
    for index, part in enumerate(parts[:-1]):
        if part == "capsem-gate":
            return parts[index + 1]
    return None


def _argv_prefix(node: ast.Call) -> tuple[str, ...]:
    """The literal leading argv of a call, stopping at the first variable.

    A prefix is enough: `["uv", "run", "capsem-gate", module]` re-enters the
    gate whatever `module` turns out to be, and refusing to reason past the
    literal part keeps this guard from guessing.
    """
    if not node.args or not isinstance(node.args[0], ast.List):
        return ()
    prefix: list[str] = []
    for element in node.args[0].elts:
        if not isinstance(element, ast.Constant) or not isinstance(element.value, str):
            break
        prefix.append(element.value)
    return tuple(prefix)


def _invocations(module: Path) -> list[tuple[int, tuple[str, ...]]]:
    """Every literal argv this module hands to a runner or an action."""
    tree = ast.parse(module.read_text(encoding="utf-8"))
    found: list[tuple[int, tuple[str, ...]]] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        name = (
            node.func.id
            if isinstance(node.func, ast.Name)
            else node.func.attr
            if isinstance(node.func, ast.Attribute)
            else ""
        )
        if name not in {"Run", "Launch", "run", "capture", "succeeds", "launch"}:
            continue
        argv = _argv_prefix(node)
        if argv:
            found.append((node.lineno, argv))
    return found


def _commands_in(module: Path) -> list[type[GateCommand]]:
    stem = module.stem
    return [
        command
        for command in GateCommand.registry.values()
        if command.__module__.rsplit(".", 1)[-1] == stem
    ]


def _modules() -> list[Path]:
    modules = sorted(GATE_PACKAGE.glob("*.py"))
    assert len(modules) > 10, "scanned too few modules to trust this guard"
    return modules


# ---------------------------------------------------------------------------
# Recursion
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("module", _modules(), ids=lambda p: p.name)
def test_no_plan_action_re_enters_the_gate(module: Path) -> None:
    """Composition happens in one process, or the machine lock deadlocks it.

    Every one of these looked reasonable at the call site -- `Run(["just",
    "_sign"])` reads like naming a step. What it does is start a second gate
    that waits out its timeout for the lock this one is holding.
    """
    offences = [
        f"{module.name}:{line}: {' '.join(argv)}"
        for line, argv in _invocations(module)
        if program(argv) in ENTRYPOINTS
    ]

    assert not offences, (
        "a plan action may not invoke just or capsem-gate; compose the other "
        "command's fragment into this plan instead:\n  " + "\n  ".join(offences)
    )


# ---------------------------------------------------------------------------
# Cross-references
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("module", _modules(), ids=lambda p: p.name)
def test_every_just_recipe_named_from_python_exists(module: Path) -> None:
    """`_build-host-image` was invoked by two modules and never existed.

    Composing the host-image fragment removed the call and the dangling name
    together, which is the usual shape: a name that resolves to nothing is
    almost always a name nobody should have been writing.
    """
    missing = [
        f"{module.name}:{line}: just {argv[1]}"
        for line, argv in _invocations(module)
        if program(argv) == "just" and len(argv) > 1 and argv[1] not in RECIPES
    ]

    assert not missing, "these recipes do not exist:\n  " + "\n  ".join(missing)


def test_every_gate_subcommand_a_recipe_dispatches_to_is_registered() -> None:
    """A recipe naming a command nobody registered fails only when run."""
    unknown = sorted(
        f"{recipe} -> capsem-gate {subcommand}"
        for recipe in RECIPES
        if (subcommand := _subcommand_of(recipe))
        and subcommand not in GateCommand.registry
    )

    assert not unknown, "these dispatch nowhere:\n  " + "\n  ".join(unknown)


def test_the_guard_can_see_the_dispatches_it_is_checking() -> None:
    """A resolver that silently matches nothing proves nothing.

    Both checks above pass trivially if `just --dump` changes shape or the
    recipes stop being readable, so the inventory itself is asserted.
    """
    dispatching = [recipe for recipe in RECIPES if _subcommand_of(recipe)]

    assert len(dispatching) > 20, (
        f"only {len(dispatching)} recipes were seen to dispatch; the resolver "
        "is probably reading the wrong thing"
    )
