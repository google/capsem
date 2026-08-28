"""The two commands that only materialize a toolchain or a node workspace.

Split from `imagebuild`, which owns building and checking guest assets and was
over the three-hundred-line ceiling. The seam is real: these two produce
nothing an asset lane consumes -- they exist so an operator can prepare one
piece of the environment without running a lane that needs all of it.
"""

from __future__ import annotations

from . import toolchain
from .command import GateCommand
from .plan import Plan


class ToolchainCommand(
    GateCommand,
    name="install-tools",
    help="install the cross-compilation targets and cargo tools a gate needs",
):
    """Idempotent: present means nothing happens, and nothing is said."""

    exclusive = True

    def plan(self) -> Plan:

        plan = Plan(self.name)
        python = plan.add(toolchain.sync(self._config))
        plan.add(toolchain.rust(self._config), after=(python,))
        plan.add(toolchain.node(self._config), after=(python,))
        return plan


class NodeCommand(
    GateCommand,
    name="install-node",
    help="install every Node workspace a local gate exercises",
):
    """Install every Node workspace that split CI jobs exercise separately."""

    exclusive = True

    def plan(self) -> Plan:

        plan = Plan(self.name)
        plan.add(toolchain.node(self._config))
        return plan
