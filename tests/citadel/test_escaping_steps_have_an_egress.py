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


def test_a_command_owning_an_escaping_step_declares_egress() -> None:
    """The declaration and the resource have to travel together."""
    import importlib

    from capsem.gate.command import GateCommand

    # Importing the CLI is what registers every command.
    importlib.import_module("capsem.gate.cli")

    # Commands whose plans compose the glow-up module, which is the one that
    # installs a package and therefore must leave the sandbox.
    for name in ("candidate", "qualify-binaries"):
        command = GateCommand.registry[name]
        assert command.outside_egress, (
            f"`{name}` composes a step that declares `outside_sandbox=True`, "
            "but holds no egress resource to honour it, so the step would run "
            "inside the sandbox and fail on sudo"
        )


def test_the_guard_has_subjects() -> None:
    """A guard over nothing asserts nothing."""
    assert _modules_using_outside_sandbox(), "no module declares outside_sandbox"
