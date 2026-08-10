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
    settings = config.assets
    other = other_architecture(config, native)
    build_arch = imagebases.build_config(config).architectures[other.name]
    platform = build_arch.docker_platform
    runner.step(f"Ironbank {other.name} container execution preflight")
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

    remedy = (
        "Colima Rosetta may be configured but stale; run 'colima restart' and retry."
        if host.on_macos()
        else "Install/register binfmt QEMU support and retry."
    )
    raise GateError(f"Docker cannot execute {platform} containers.\n{remedy}")
