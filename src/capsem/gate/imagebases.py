"""Exact container bases for guest image builds.

The Docker daemon is the declared container-fetch boundary.  The gate itself
stays inside its kernel sandbox, asks the daemon for immutable child manifests,
then runs every image build from those locally materialized inputs.
"""

from __future__ import annotations

from collections.abc import Iterable

from capsem.builder.config import load_guest_config
from capsem.builder.models import ArchConfig, BuildConfig

from .actions import Action
from .config import GateConfig
from .context import Context
from .docker import Docker
from .errors import GateError
from .proc import Runner


def build_config(config: GateConfig) -> BuildConfig:
    """Load the profile-materialization source through its product schema."""
    build = load_guest_config(config.path(config.imagebuild.source_config)).build
    missing = sorted(set(config.architectures) - set(build.architectures))
    if missing:
        raise GateError(f"guest image config has no base for: {', '.join(missing)}")
    for name, gate_arch in config.architectures.items():
        platform = build.architectures[name].docker_platform
        expected = f"linux/{gate_arch.dpkg}"
        if platform != expected:
            raise GateError(
                f"guest image architecture {name} uses {platform}, expected {expected}"
            )
    return build


def selected(
    config: GateConfig, names: Iterable[str] | None = None
) -> tuple[tuple[str, ArchConfig], ...]:
    """Return validated base inputs in deterministic architecture order."""
    build = build_config(config)
    wanted = tuple(names) if names is not None else tuple(config.architectures)
    unknown = sorted(set(wanted) - set(build.architectures))
    if unknown:
        raise GateError(f"guest image config has no base for: {', '.join(unknown)}")
    return tuple((name, build.architectures[name]) for name in wanted)


def prefetch(runner: Runner, config: GateConfig, names: Iterable[str] | None = None) -> None:
    """Pull each absent immutable child manifest for its exact platform."""
    docker = Docker(runner)
    for name, arch in selected(config, names):
        if docker.image_exists(arch.base_image, platform=arch.docker_platform):
            runner.note(f"exact {name} guest base is already present: {arch.base_image}")
            continue
        docker.pull(arch.base_image, platform=arch.docker_platform)


class Prefetch(Action, name="guest-base-prefetch"):
    """A visible, timed cold-host boundary shared by every asset rail."""

    def __init__(self, names: Iterable[str] | None = None) -> None:
        self._names = tuple(names) if names is not None else None

    def render(self) -> str:
        scope = "all architectures" if self._names is None else ", ".join(self._names)
        return f"materialize exact guest base images ({scope})"

    def perform(self, context: Context) -> None:
        prefetch(context.runner, context.config, self._names)
