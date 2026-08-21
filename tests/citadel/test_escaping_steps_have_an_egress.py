"""A step that must leave the sandbox needs a command that can take it out.

`outside_sandbox=True` on an action only means something if the command
running it holds an `Egress` resource. The context used to fall back to the
sandboxed runner when it held none, so the declaration silently did nothing.

`glowup.package` is what that cost. It installs a system package, its own
comment names `PR_SET_NO_NEW_PRIVS` as the reason it must be outside, and it
ran inside anyway -- surfacing two hours into a release lane as
`sudo: /etc/sudo.conf is owned by uid 65534, should be 0`, a message naming
neither the step nor the sandbox. The local lane holds the resource, so no
local run could reproduce it.
"""

from __future__ import annotations

import ast
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
GATE = PROJECT_ROOT / "src" / "capsem" / "gate"


def _modules_using_outside_sandbox() -> set[str]:
    """Gate modules that build an action declaring it escapes the sandbox."""
    found = set()
    for source in sorted(GATE.glob("*.py")):
        tree = ast.parse(source.read_text(encoding="utf-8"))
        for node in ast.walk(tree):
            if not isinstance(node, ast.keyword) or node.arg != "outside_sandbox":
                continue
            if isinstance(node.value, ast.Constant) and node.value.value is True:
                found.add(source.stem)
    return found


def test_the_gate_refuses_rather_than_running_it_inside() -> None:
    """The fallback that hid this must stay gone."""
    escape = (GATE / "escape.py").read_text(encoding="utf-8")
    assert "def escaping_runner" in escape
    context = (GATE / "context.py").read_text(encoding="utf-8")
    assert "return self.outside_runner or self.runner" not in context, (
        "the silent fallback is back: a step that declared it must escape the "
        "sandbox would run inside it again, and say nothing"
    )


def test_every_action_that_escapes_goes_through_the_refusing_accessor() -> None:
    offenders = []
    for source in sorted(GATE.glob("*.py")):
        text = source.read_text(encoding="utf-8")
        if "context.external_runner" in text:
            offenders.append(source.name)
    assert not offenders, (
        "these read the old permissive accessor, which falls back to the "
        f"sandboxed runner instead of refusing: {offenders}"
    )


def _commands_whose_plans_escape() -> dict[str, bool]:
    """Every command whose plan contains an escaping action, and its declaration.

    Derived by building each plan rather than listing names. Naming two of them
    by hand is what let `cross-compile` through: it builds the Linux host
    image, which declares `outside_sandbox=True`, and held no egress -- so both
    Linux release builds failed on a refusal the guard had not thought to ask
    about.
    """
    import importlib

    from capsem.gate.command import GateCommand

    importlib.import_module("capsem.gate.cli")
    from helpers.gate import gate_plan

    found = {}
    for name, command in sorted(GateCommand.registry.items()):
        try:
            plan = gate_plan(name)
        except Exception:
            # A command needing arguments cannot be built here; the ones that
            # own escaping steps in a release lane all can.
            continue
        if "[outside kernel sandbox]" in plan.describe():
            found[name] = bool(command.outside_egress)
    return found


def test_every_command_owning_an_escaping_step_declares_egress() -> None:
    escaping = _commands_whose_plans_escape()
    assert escaping, "no command's plan contains an escaping action"

    undeclared = sorted(name for name, declared in escaping.items() if not declared)
    assert not undeclared, (
        "these commands compose a step declaring `outside_sandbox=True` but "
        "hold no egress resource to honour it, so the step refuses at runtime "
        f"instead of escaping: {undeclared}"
    )


def test_the_guard_has_subjects() -> None:
    """A guard over nothing asserts nothing."""
    assert _modules_using_outside_sandbox(), "no module declares outside_sandbox"
