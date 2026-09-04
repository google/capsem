"""Process-lifetime leases for policy-owned cache generations."""

from __future__ import annotations

import fcntl
import os
from pathlib import Path
from typing import TYPE_CHECKING, BinaryIO

if TYPE_CHECKING:
    from .paths import CachePaths

_HELD: dict[Path, BinaryIO] = {}


def retain_path(path: Path) -> BinaryIO:
    """Hold one shared lease until explicit release or process exit."""
    lease = path.absolute()
    existing = _HELD.get(lease)
    if existing is not None and not existing.closed:
        return existing
    lease.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.fdopen(os.open(lease, os.O_APPEND | os.O_CREAT | os.O_RDWR, 0o600), "a+b")
    try:
        fcntl.flock(descriptor, fcntl.LOCK_SH | fcntl.LOCK_NB)
    except BaseException:
        descriptor.close()
        raise
    _HELD[lease] = descriptor
    return descriptor


def retain_generation(paths: CachePaths, stage_id: str, key: str) -> BinaryIO:
    """Hold the configured lease for one managed stage generation."""
    stage = paths.policy.stages[stage_id]
    if stage.lease_template is None:
        raise ValueError(f"cache stage {stage_id!r} has no generation lease")
    return retain_path(paths.stage(stage_id) / stage.lease_template.format(key=key))


def release_path(path: Path) -> None:
    """Release a retained path, primarily for bounded test fixtures."""
    descriptor = _HELD.pop(path.absolute(), None)
    if descriptor is not None:
        descriptor.close()


def active_path(path: Path) -> bool:
    """Return whether another process holds a shared lease at this path."""
    lease = path.absolute()
    if not lease.is_file() or lease.is_symlink():
        return False
    with lease.open("rb") as descriptor:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return True
        fcntl.flock(descriptor, fcntl.LOCK_UN)
    return False
