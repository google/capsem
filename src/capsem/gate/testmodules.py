"""The modules `just test` is made of, as graphs over shared components.

`_test-candidate-run` was 366 lines because six modules each re-solved the same
problems inside one `bash` body, selected by a `CAPSEM_TEST_MODULE` environment
variable and a `module_enabled` function. A module was a region of a file
between two `if` statements, which is why running one in isolation meant
setting an environment variable and hoping, and why nothing could say what one
would do without doing it.

Each is a command now. It declares the workspace it needs and the graph of
steps it contains, and both are answerable for free.
"""

from __future__ import annotations

from . import audits, toolchain
from .command import GateCommand
from .config import GateConfig
from .lifecycle import Resource
from .plan import Plan
from .workspace import Workspace


class InWorkspace:
    """Runs against an isolated `CAPSEM_HOME`, never the developer's.

    A mixin rather than a base command, because a base would have to register
    itself as a runnable name and there is nothing to run.
    """

    exclusive = True

    _config: GateConfig
    """Supplied by `GateCommand`, declared here so the mixin type-checks."""

    def resources(self) -> tuple[Resource, ...]:
        return (Workspace(self._config),)


class FastModule(
    InWorkspace,
    GateCommand,
    name="test-fast",
    help="the checks that fail in minutes rather than in forty",
):
    """Cheap, independent, and the most common failure class.

    Everything here is free to overlap except one edge: clippy reads
    `frontend/dist`, which `capsem-app` embeds at compile time, so the frontend
    build must finish first. The shell expressed that as a conditional which
    skipped clippy entirely when the frontend failed -- losing the clippy
    result on exactly the runs where the most had changed.
    """

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config

        # The environment first: everything below runs through uv or pnpm,
        # and a gate that assumes the lockfile is already installed is a
        # gate that works on the machine it was written on.
        python = plan.add(toolchain.sync(config))
        node = plan.add(toolchain.node(config), after=(python,))
        rust = plan.add(toolchain.rust(config), after=(python,))

        # Nothing is worth starting against a file that will not parse.
        syntax = plan.add(audits.source_syntax(config), after=(python,))

        for check in audits.all_of(config):
            plan.add(check, after=(syntax,))
        plan.add(audits.lint(config), after=(syntax,))

        surfaces = [
            plan.add(surface, after=(syntax, node)) for surface in audits.web_surfaces(config)
        ]
        plan.add(
            audits.clippy(config),
            after=(audits.blocking_surface(config, surfaces), rust),
        )
        return plan
