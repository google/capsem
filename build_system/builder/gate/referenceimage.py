"""The reference image as an input of the installed proofs.

`[functional.reference_image]` pins the official `dev` image every runtime test
boots. The installed proofs -- winterfell, run by the glow-up and by the
transition replay -- boot it too, from a container or host that cannot read
the gate's cache: they get the verified layout's path in one variable, and the
container mounts that path read-only where it is.
"""

from __future__ import annotations

from pathlib import Path

from . import cachelayout
from .actions import Script
from .config import GateConfig
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step


def layout(config: GateConfig) -> Path | None:
    """Where the pinned layout for this host's platform lives in its stage, or
    None for a platform with no pin yet (amd64 until a Linux x86_64 host builds
    one). The plan still builds there; the installed proof then refuses at run
    time, naming the missing layout, instead of passing without the image."""
    settings = config.functional.reference_image
    digest = settings.digests.get(config.host_arch().docker_platform)
    if digest is None:
        return None
    stage = cachelayout.stage_path(config, settings.cache_stage)
    return stage / f"{settings.name}-{digest.removeprefix('sha256:')}"


def inputs(config: GateConfig) -> tuple[Path, ...]:
    """The layout as an install container's read-only input, when pinned."""
    found = layout(config)
    return () if found is None else (found,)


def environment(config: GateConfig) -> dict[str, str]:
    """The variable an installed proof reads the layout from, when pinned."""
    variable = config.functional.reference_image.layout_variable
    if variable is None:
        raise GateError("[functional.reference_image] names no layout_variable")
    found = layout(config)
    return {} if found is None else {variable: str(found)}


def prepare(config: GateConfig) -> Step:
    """Verify the pinned layout, pulling it by digest only when absent."""
    settings = config.functional.reference_image
    return step(
        "reference-image",
        Script(
            config,
            settings.script,
            "reference_image",
            "prepare",
            "--platform",
            config.host_arch().docker_platform,
            outside_sandbox=True,
        ),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK, Needs.NETWORK}),
        speed=Speed.FAST,
    )
