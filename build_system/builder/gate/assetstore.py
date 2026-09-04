"""Place reusable VM assets behind the typed cache-owned location."""

from __future__ import annotations

from pathlib import Path

from . import cachelayout
from .config import Arch, GateConfig
from .errors import GateError
from .filesystem import link, make_dir, remove


def root(config: GateConfig) -> Path:
    """Return the stable generation root selected by the cache policy."""
    stage = cachelayout.stage_policy(config, "assets")
    return cachelayout.stage_path(config, "assets") / stage.entry_root


def lane(config: GateConfig, identity: str, *, profile: str, arch: Arch) -> Path:
    """Resolve one content-addressed profile/architecture product."""
    if len(identity) != 64 or identity.strip("0123456789abcdef"):
        raise GateError(f"VM image cache identity {identity!r} is not a canonical digest")
    if not profile or Path(profile).name != profile:
        raise GateError(f"VM image cache profile {profile!r} is not one plain name")
    return root(config) / identity / profile / f"build-{arch.name}"


def _local_lane(config: GateConfig, *, profile: str, arch: Arch) -> Path:
    return config.path(config.assets.test_root) / profile / f"build-{arch.name}"


def _link_lane(local: Path, cached: Path) -> None:
    make_dir(cached)
    make_dir(local.parent)
    if local.is_symlink() and local.resolve() == cached.resolve():
        return
    remove(local)
    link(local, str(cached))


def materialize(config: GateConfig, profiles: tuple[str, ...], identity: str) -> None:
    """Point this prefix at its exact generation, migrating old local output."""
    for profile in profiles:
        for arch in config.architectures.values():
            local = _local_lane(config, profile=profile, arch=arch)
            cached = lane(config, identity, profile=profile, arch=arch)
            if local.is_dir() and not local.is_symlink() and not cached.exists():
                make_dir(cached.parent)
                try:
                    local.rename(cached)
                except OSError as error:
                    raise GateError(
                        f"cannot migrate VM asset generation {local} to {cached}; "
                        "both locations must share a filesystem"
                    ) from error
            _link_lane(local, cached)


def reset_lane(config: GateConfig, local: Path, identity: str, *, profile: str, arch: Arch) -> None:
    """Discard one invalid product and recreate its stable selector."""
    cached = lane(config, identity, profile=profile, arch=arch)
    remove(local)
    remove(cached)
    _link_lane(local, cached)
