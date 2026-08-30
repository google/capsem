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
from enum import StrEnum
from pathlib import Path

from . import host
from .actions import Action, Run, Script
from .config import GateConfig
from .context import Context
from .errors import GateError
from .escape import escaping_runner
from .execution import Kind, Needs, Speed, Step, step
from .packageinputs import pinned_toolchain


class OrtConsumer(StrEnum):
    """Rust build cohorts that must never replace each other's selected bytes."""

    FAST = "fast"
    STATIC = "static"


def _ort_target(config: GateConfig) -> str:
    system = host.system()
    architecture = host.machine()
    try:
        return config.toolchain.ort.host_targets[system][architecture]
    except KeyError as error:
        raise GateError(f"toolchain.ort has no distribution for {system}/{architecture}") from error


def _ort_paths(config: GateConfig, consumer: OrtConsumer) -> tuple[Path, Path]:
    settings = config.toolchain.ort
    target = _ort_target(config)
    distribution = settings.distributions[target]
    fields = {"consumer": consumer.value, "target": target, "sha256": distribution.sha256}
    return (
        Path(settings.archive_cache_template.format(**fields)).expanduser(),
        config.path(settings.output_template.format(**fields)),
    )


def ort_environment(config: GateConfig, consumer: OrtConsumer) -> dict[str, str]:
    """Select the exact static ORT bytes materialized for this host."""
    _archive, output = _ort_paths(config, consumer)
    settings = config.toolchain.ort
    return {
        settings.strategy_variable: settings.strategy,
        settings.location_variable: str(output),
    }


def ort(config: GateConfig, consumer: OrtConsumer) -> Step:
    """Materialize ort-sys inputs before entering a sealed Rust build."""
    settings = config.toolchain.ort
    target = _ort_target(config)
    distribution = settings.distributions[target]
    archive, output = _ort_paths(config, consumer)
    return step(
        "toolchain.ort",
        Script(
            config,
            settings.script,
            "--url",
            distribution.url,
            "--sha256",
            distribution.sha256,
            "--archive-cache",
            archive,
            "--replace",
            "--output",
            output,
            outside_sandbox=True,
        ),
        produces=(output / "libonnxruntime.a",),
        kind=Kind.COMPILE,
        # `outside_sandbox`: it really does fetch a pinned distribution.
        needs=frozenset({Needs.NETWORK, Needs.DISK}),
        speed=Speed.FAST,
    )


def sync(config: GateConfig) -> Step:
    """The Python environment, from the lockfile."""
    return step(
        "toolchain.python",
        Run(config.toolchain.sync),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    )


def node(config: GateConfig, workspaces: tuple[str, ...] | None = None) -> Step:
    """Every Node workspace a local gate exercises, or the named subset.

    CI has separate jobs for docs, site and release-site; a local `just test-full`
    builds all of them in this one checkout, so all of them are installed here.

    A caller that needs one workspace says so. `pnpm install` reaches the
    registry for anything its store does not already hold, and a release lane
    runs inside a network namespace with only loopback -- so installing a
    workspace nobody warmed is not a slow no-op, it is a failure.
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
            for workspace in (workspaces or settings.node_workspaces)
        ],
        # `pnpm install` rewrites a workspace's node_modules in place, and
        # every web build reads it. Two installs overlapping, or an install
        # overlapping a build, is a torn tree either way.
        contends=(config.exclusive("node_modules"),),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    )


def rust(config: GateConfig) -> Step:
    """Cross-compilation targets, components, and the cargo-installed tools.

    Each is guarded by its own probe, so a machine that already has them does
    no work and says nothing.
    """
    return step(
        "toolchain.rust",
        _EnsureRust(),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.NETWORK, Needs.DISK}),
        speed=Speed.FAST,
    )


class _EnsureRust(Action, name="ensure-rust"):
    """Probe-then-install, driven by the configured tables."""

    outside_sandbox = True

    def render(self) -> str:
        return (
            "probe rustup targets/components and cargo tools; materialize missing "
            "pinned items [outside kernel sandbox]"
        )

    def perform(self, context: Context) -> None:
        settings = context.config.toolchain
        toolchain = pinned_toolchain(context.config.root)
        installer = escaping_runner(context, "materialize the pinned Rust toolchain")

        installed = context.runner.capture(
            ["rustup", "target", "list", "--toolchain", toolchain, "--installed"]
        )
        for target in settings.rust_targets:
            if target not in installed:
                installer.run(["rustup", "target", "add", "--toolchain", toolchain, target])

        components = context.runner.capture(
            ["rustup", "component", "list", "--toolchain", toolchain, "--installed"]
        )
        for component in settings.rust_components:
            if component not in components:
                installer.run(
                    ["rustup", "component", "add", "--toolchain", toolchain, component]
                )

        for crate in settings.crates:
            # `shutil.which`, not `command -v`: the latter is a shell builtin
            # and there is no shell here, so it would report every tool
            # missing and reinstall the world on each run.
            actual = ""
            if shutil.which(crate.name) is not None:
                actual = context.runner.capture(crate.probe, check=False)
            if not actual.startswith(crate.expected):
                installer.run(crate.install)
                actual = context.runner.capture(crate.probe, check=False)
                if not actual.startswith(crate.expected):
                    raise GateError(
                        f"{crate.name} did not provide {crate.expected}: "
                        f"{actual or '<no version output>'}"
                    )
