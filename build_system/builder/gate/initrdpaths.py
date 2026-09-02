"""Where an initrd and its staging tree live, and whether it is stale.

Shared by the module that composes initrd steps and the one holding the actions
those steps run. Neither may import the other for them -- that is a cycle -- so
they live here.
"""

from __future__ import annotations

from pathlib import Path

from ..image import guestbinarycache
from . import imagebases
from .config import GateConfig
from .errors import GateError


def staging_for(config: GateConfig, arch: str | None = None) -> Path:
    selected = config.arch(arch).name if arch else config.host_arch().name
    return config.path(config.initrd.staging) / selected


def initrd_target(config: GateConfig, arch: str | None = None) -> Path:
    """Where the repack writes, whether or not it is there yet."""
    selected = config.arch(arch).name if arch else config.host_arch().name
    return config.path(config.imagebuild.output) / selected / config.artifacts.initrd


def initrd_at(config: GateConfig, arch: str | None = None) -> Path:
    found = initrd_target(config, arch)
    if not found.is_file():
        raise GateError(f"initrd not found at {found}; run `just doctor fix` first")
    return found


def needs_rebuild(config: GateConfig, arch: str | None = None) -> bool:
    """Whether staging differs from the current content-addressed generation."""
    selected = config.arch(arch).name if arch else config.host_arch().name
    return not guestbinarycache.current(
        imagebases.build_config(config),
        selected,
        config.root,
        staging_for(config, selected),
        config.initrd.binaries,
    )
