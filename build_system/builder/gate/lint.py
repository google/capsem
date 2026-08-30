"""Lint and type-check every line of Python in the repository.

`ruff check .` always covered the whole tree. `ty` did not: it ran on the
retired Python source root and nothing else, so release machinery and every
test helper went unchecked. A type error in
`build_system/scripts/release/release-binaries.py` is a release bug; it had no gate at all.

Which trees are checked, which are checked strictly, and which rules are held
back on the rest are all `[lint]` in `config/gate.toml`. The strict source
owners pass every Ty rule with none disabled; the other trees hold back the
`ty_ratchet` list -- roughly four hundred diagnostics dominated by inference
over untyped fixture data, which would otherwise force the choice between
checking those trees loosely and not checking them at all. That is the choice
that left them unchecked. Entries may leave the ratchet; nothing may join it.
"""

from __future__ import annotations

from . import sourcechecks
from .command import GateCommand
from .plan import Plan


class LintCommand(GateCommand, name="lint", help="ruff and ty over every first-party Python tree"):
    """The same fragment the fast phase composes, on its own.

    It was one opaque `Call` around a function that ran the tools in sequence
    and gathered their failures into a list by hand -- which is what a plan
    does for every other set of independent steps, done once, somewhere the
    graph cannot see.
    """

    def plan(self) -> Plan:
        plan = Plan(self.name)
        sourcechecks.fragment(plan, self._config)
        return plan
