"""Content-addressed, bounded storage for reusable VM boot images."""

from __future__ import annotations

import os
import time
from pathlib import Path

from capsem.cachepolicy import CacheLimits, CacheProduct, plan_reclaim

from . import assetreceipt, prefix
from .config import Arch, GateConfig
from .errors import GateError
from .filesystem import link, make_dir, remove


def root(config: GateConfig) -> Path:
    """The stable VM image cache beside the ephemeral prefix root."""
    return Path(config.prefix.vm_image_cache.format(parent=config.prefix.parent)).expanduser()


def lane(config: GateConfig, identity: str, *, profile: str, arch: Arch) -> Path:
    """One content-addressed profile/architecture product."""
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
    """Point this prefix at its exact generation, migrating the old local cache."""
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
                        f"cannot migrate VM image cache {local} to {cached}; "
                        "prefix and vm_image_cache must share a filesystem"
                    ) from error
            _link_lane(local, cached)


def reset_lane(
    config: GateConfig, local: Path, identity: str, *, profile: str, arch: Arch
) -> None:
    """Discard one invalid generation product and recreate its stable selector."""
    cached = lane(config, identity, profile=profile, arch=arch)
    remove(local)
    remove(cached)
    _link_lane(local, cached)


def _cached_lanes(config: GateConfig) -> tuple[Path, ...]:
    cache = root(config)
    if not cache.is_dir() or cache.is_symlink():
        return ()
    return tuple(
        sorted(
            candidate
            for generation in cache.iterdir()
            if generation.is_dir() and not generation.is_symlink()
            for profile in generation.iterdir()
            if profile.is_dir() and not profile.is_symlink()
            for candidate in profile.iterdir()
            if candidate.is_dir()
            and not candidate.is_symlink()
            and candidate.name.startswith("build-")
        )
    )


def _retained_prefix_lanes(config: GateConfig) -> frozenset[Path]:
    """Products selected by other owned prefixes are not eviction candidates."""
    selected: set[Path] = set()
    parent = prefix.parent_dir(config)
    if not parent.is_dir():
        return frozenset()
    cache = root(config).resolve()
    for checkout in parent.iterdir():
        if (
            checkout.resolve() == config.root.resolve()
            or not checkout.is_dir()
            or checkout.is_symlink()
            or checkout.stat().st_uid != os.getuid()
        ):
            continue
        for selector in (checkout / config.assets.test_root).glob("*/build-*"):
            if selector.is_symlink() and selector.resolve().is_relative_to(cache):
                selected.add(selector.resolve())
    return frozenset(selected)


def enforce(config: GateConfig, *, protected: frozenset[Path]) -> tuple[Path, ...]:
    """Apply count, age, and byte bounds; pinned overflow is a hard failure."""
    now = time.time()
    pinned = {
        path.resolve() for path in protected | _retained_prefix_lanes(config)
    }
    products: list[CacheProduct] = []
    by_key: dict[str, Path] = {}
    for path in _cached_lanes(config):
        key = path.relative_to(root(config)).as_posix()
        metadata = assetreceipt.cache_metadata(config, path)
        if metadata is None:
            stat = path.stat()
            created = last_used = stat.st_mtime
            size = _tree_size(path)
        else:
            created, last_used, size = metadata
        products.append(CacheProduct(key, size, created, last_used, path.resolve() in pinned))
        by_key[key] = path
    policy = config.assets.cache
    plan = plan_reclaim(
        tuple(products),
        CacheLimits(
            policy.maximum_count,
            policy.maximum_age_hours * 3600,
            policy.maximum_bytes,
        ),
        now=now,
    )
    removed = tuple(by_key[key] for key in plan.evict)
    for path in removed:
        remove(path)
        _prune_empty(path.parent, stop=root(config))
    if plan.violations:
        raise GateError(
            "VM image cache cannot meet count/age/byte policy without evicting "
            "active or resumable lanes: " + "; ".join(plan.violations)
        )
    return removed


def _tree_size(path: Path) -> int:
    return sum(
        candidate.lstat().st_size
        for candidate in path.rglob("*")
        if candidate.is_file() and not candidate.is_symlink()
    )


def _prune_empty(path: Path, *, stop: Path) -> None:
    while path != stop and path.is_dir() and not any(path.iterdir()):
        remove(path)
        path = path.parent


def clean(config: GateConfig) -> None:
    """Explicit aggressive cleanup of the config-owned cache root."""
    remove(root(config))


def footprint(config: GateConfig) -> int:
    """Bytes retained outside the checkout, for the GC survey."""
    cache = root(config)
    return _tree_size(cache) if cache.is_dir() and not cache.is_symlink() else 0
