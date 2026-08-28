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
from capsem_builder.gate import cli  # noqa: F401 - imported so every command registers
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.command import GateCommand
from capsem_builder.gate.funnel import ENTRYPOINTS, program

PROJECT_ROOT = Path(__file__).resolve().parents[3]
GATE_PACKAGE = PROJECT_ROOT / "build_system" / "builder" / "gate"

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
            "".join(part for part in line if isinstance(part, str)) for line in body["body"]
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


#: The one module that re-enters the gate on purpose, and may.
#:
#: The rule here is about *plan actions*: a step that shells out to the gate
#: starts a second one that waits out its timeout for the lock the first is
#: holding. `prefix` is not a step. It runs from `reexec()`, above every
#: resource and before the machine lock exists, and the process it starts is
#: the one that goes on to take that lock -- exactly once, because the child
#: inherits a marker that stops it building a copy of its own.
#:
#: Narrow on purpose: the exemption is the module, and the reason it is safe is
#: *where* it runs. If any of this moves inside a plan, the deadlock it
#: prevents is real, so the guard must go back to covering it.
RE_EXEC_BEFORE_THE_LOCK = {"prefix.py"}


@pytest.mark.parametrize("module", _modules(), ids=lambda p: p.name)
def test_no_plan_action_re_enters_the_gate(module: Path) -> None:
    """Composition happens in one process, or the machine lock deadlocks it.

    Every one of these looked reasonable at the call site -- `Run(["just",
    "_sign"])` reads like naming a step. What it does is start a second gate
    that waits out its timeout for the lock this one is holding.
    """
    if module.name in RE_EXEC_BEFORE_THE_LOCK:
        pytest.skip(f"{module.name} re-execs before the lock is taken, not from a plan")

    offences = [
        f"{module.name}:{line}: {' '.join(argv)}"
        for line, argv in _invocations(module)
        if program(argv) in ENTRYPOINTS
    ]

    assert not offences, (
        "a plan action may not invoke just or capsem-gate; compose the other "
        "command's fragment into this plan instead:\n  " + "\n  ".join(offences)
    )


def test_the_re_exec_exemption_is_not_reachable_from_a_plan() -> None:
    """What `RE_EXEC_BEFORE_THE_LOCK` costs, paid back.

    Skipping the argv scan for `prefix.py` is only safe while its re-exec
    genuinely cannot happen inside a plan. So this asserts the thing the skip
    assumes: nothing that builds or runs steps reaches it. `command.py` is the
    single caller, from `execute` before any resource is acquired.

    Without this, the exemption is a hole shaped exactly like the deadlock the
    guard exists to prevent -- a step calling into `prefix` would start a gate
    that waits out its timeout for the lock its own parent holds, and the scan
    that would have caught it has been told not to look.
    """
    # The entry point that spawns a gate, not the module. `sourcestate` calls
    # `prefix.source_checkout` from inside a plan action and always will --
    # that one reads an environment variable and starts nothing.
    callers = sorted(
        module.name
        for module in _modules()
        if module.name not in {"prefix.py", "command.py"}
        and "run_from_private_copy" in module.read_text(encoding="utf-8")
    )
    assert not callers, (
        "these reach the prefix re-exec, which is exempt from the recursion "
        f"scan only because `command.py` is its one caller: {callers}"
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
        if (subcommand := _subcommand_of(recipe)) and subcommand not in GateCommand.registry
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
