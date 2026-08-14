"""Where an initrd and its staging tree live, and whether it is stale.

Shared by the module that composes initrd steps and the one holding the actions
those steps run. Neither may import the other for them -- that is a cycle -- so
they live here.
"""

from __future__ import annotations

from pathlib import Path

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
    """Whether any staged guest binary is missing or older than its inputs.

    Its *inputs*, not just its `*.rs` files. A dependency bump, a feature
    change or a toolchain bump leaves every source file older than the staged
    binary while the binary is stale -- and a stale guest binary ships into an
    initrd that does not match the source it claims to have been built from.
    """
    settings = config.initrd
    staged = [staging_for(config, arch) / name for name in settings.binaries]
    if any(not path.is_file() for path in staged):
        return True

    oldest = min(path.stat().st_mtime for path in staged)
    return any(source.stat().st_mtime > oldest for source in _build_inputs(config))


def _build_inputs(config: GateConfig):
    """Every file whose change should invalidate the staged binaries."""
    settings = config.initrd
    for source_root in settings.sources:
        for pattern in settings.freshness_globs:
            yield from config.path(source_root).rglob(pattern)
    for relative in settings.freshness_inputs:
        candidate = config.path(relative)
        if candidate.is_file():
            yield candidate
