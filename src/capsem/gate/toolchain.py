"""Getting the machine ready, idempotently.

`_install-tools` was eight near-identical shell blocks: probe for a thing, and
install it if the probe failed. The only parts that varied were the probe and
the install, so eight blocks existed to express two values -- which is why a
ninth tool meant another block rather than another line.

Everything here is safe to repeat. A gate that has to know whether it is the
first run on a machine is a gate with two behaviours and one of them untested.
"""

from __future__ import annotations

import shutil

from .actions import Action, Run
from .config import GateConfig
from .context import Context
from .execution import Step, step


def sync(config: GateConfig) -> Step:
    """The Python environment, from the lockfile."""
    return step("toolchain.python", Run(config.toolchain.sync))


def node(config: GateConfig) -> Step:
    """Every Node workspace a local gate exercises.

    CI has separate jobs for docs, site and release-site; a local `just test`
    builds all of them in this one checkout, so all of them are installed here.
    """
    settings = config.toolchain
    return step(
        "toolchain.node",
        *[
            Run(
                settings.node_install,
                cwd=config.path(workspace),
                env=dict(settings.node_env),
            )
            for workspace in settings.node_workspaces
        ],
        # `pnpm install` rewrites a workspace's node_modules in place, and
        # every web build reads it. Two installs overlapping, or an install
        # overlapping a build, is a torn tree either way.
        contends=(config.exclusive("node_modules"),),
    )


def rust(config: GateConfig) -> Step:
    """Cross-compilation targets, components, and the cargo-installed tools.

    Each is guarded by its own probe, so a machine that already has them does
    no work and says nothing.
    """
    return step("toolchain.rust", _EnsureRust())


class _EnsureRust(Action, name="ensure-rust"):
    """Probe-then-install, driven by the configured tables."""

    def render(self) -> str:
        return "rustup targets/components and the cargo tools, if any are missing"

    def perform(self, context: Context) -> None:
        settings = context.config.toolchain

        installed = context.runner.capture(["rustup", "target", "list", "--installed"])
        for target in settings.rust_targets:
            if target not in installed:
                context.runner.run(["rustup", "target", "add", target])

        components = context.runner.capture(["rustup", "component", "list", "--installed"])
        for component in settings.rust_components:
            if component not in components:
                context.runner.run(["rustup", "component", "add", component])

        for crate in settings.crates:
            # `shutil.which`, not `command -v`: the latter is a shell builtin
            # and there is no shell here, so it would report every tool
            # missing and reinstall the world on each run.
            if shutil.which(crate.name) is None:
                context.runner.run(crate.install)
