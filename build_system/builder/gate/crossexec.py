"""Prove Docker can execute the architecture we are about to build for.

Its own module because it is its own question: `assets` builds VM assets, and
whether this daemon can run a foreign-architecture container is a property of
the machine, asked once before any of that starts.

Asked *first* for a blunt reason -- discovering that Rosetta is stale after
twenty minutes of image builds wastes the twenty minutes.
"""

from __future__ import annotations

from . import config as gate_config
from . import host, imagebases
from .actions import Action
from .context import Context
from .docker import Docker
from .errors import GateError
from .proc import Runner


def other_architecture(config: gate_config.GateConfig, native: gate_config.Arch):
    """The architecture this host is not."""
    others = [arch for arch in config.architectures.values() if arch.name != native.name]
    if len(others) != 1:
        raise GateError(
            "the asset gate expects exactly one non-host architecture, got "
            f"{[arch.name for arch in others]}"
        )
    return others[0]


def require(runner: Runner, config: gate_config.GateConfig, native: gate_config.Arch) -> None:
    """Refuse to start if the daemon cannot run the other architecture."""
    other = other_architecture(config, native)
    require_architecture(runner, config, native, other)


def require_architecture(
    runner: Runner,
    config: gate_config.GateConfig,
    native: gate_config.Arch,
    target: gate_config.Arch,
) -> None:
    """Refuse before target-platform Dockerfile layers if they cannot execute."""
    settings = config.assets
    build_arch = imagebases.build_config(config).architectures[target.name]
    platform = build_arch.docker_platform
    runner.step(f"Ironbank {target.name} container execution preflight")
    # `--network none`: the probe runs `/bin/true` to find out whether the
    # daemon can execute the other architecture at all. Nothing it does needs
    # the network, and saying so is what the wrapper requires of every
    # container rather than letting one omit the decision.
    if Docker(runner).probe(
        image=build_arch.base_image,
        command=[settings.cross_platform_probe_command],
        network=settings.cross_platform_probe_network,
        options=("--platform", platform),
    ):
        return

    if target.name == native.name:
        remedy = "Restart the Docker daemon (or Colima on macOS) and retry."
    elif host.on_macos():
        remedy = "Colima Rosetta may be configured but stale; run 'colima restart' and retry."
    else:
        remedy = "Run './bootstrap.sh --yes' outside the gate to install/register binfmt QEMU."
    raise GateError(f"Docker cannot execute {platform} containers.\n{remedy}")


class Require(Action, name="container-execution-require"):
    """Plan-visible execution proof for the architectures a standalone rail uses."""

    def __init__(self, names: tuple[str, ...]) -> None:
        self._names = names

    def render(self) -> str:
        return f"prove Docker can execute {', '.join(self._names)} containers"

    def perform(self, context: Context) -> None:
        config = context.config
        native = config.host_arch()
        for name in self._names:
            require_architecture(context.runner, config, native, config.arch(name))
