"""Cross-process ownership for deterministic private source directories."""

from __future__ import annotations

import fcntl
import os
from contextlib import contextmanager
from pathlib import Path

from . import cachelayout
from .config import GateConfig
from .errors import GateError, PrefixBusy


def parent_dir(config: GateConfig) -> Path:
    return cachelayout.shared_path(config, config.prefix.parent)


@contextmanager
def lease(config: GateConfig, path: Path):
    """Hold the nonblocking cross-process lease for one direct child."""
    root_path = parent_dir(config)
    if root_path.is_symlink():
        raise GateError(f"prefix parent {root_path} must not be a symlink")
    root_path.mkdir(parents=True, exist_ok=True)
    root = root_path.resolve()
    if root.stat().st_uid != os.getuid():
        raise GateError(f"prefix parent {root} is not owned by the current user")
    resolved = Path(os.path.abspath(path))
    if resolved.parent != root:
        raise GateError(f"prefix lease target {resolved} is not a direct child of {root}")
    name = config.prefix.lease_template.format(identity=resolved.name)
    with (root / name).open("a+", encoding="utf-8") as handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise PrefixBusy(f"prefix {resolved} is already in use") from error
        try:
            yield
        finally:
            fcntl.flock(handle, fcntl.LOCK_UN)


def _identity_of(config: GateConfig, name: str) -> str | None:
    """The prefix a lease file names, or None if the file is not one."""
    head, _, tail = config.prefix.lease_template.partition("{identity}")
    if not name.startswith(head) or not name.endswith(tail):
        return None
    identity = name[len(head) : len(name) - len(tail)] if tail else name[len(head) :]
    return identity or None


def reclaim_orphan_leases(config: GateConfig) -> list[Path]:
    """Remove lease files whose prefix is gone, and say which went.

    One is created per identity ever run and nothing removed them; 127 had
    accumulated here. Zero bytes each, so this is not about space -- it is that
    the directory holding the prefixes stops being readable at a glance, and
    that listing is where a prefix nobody reclaimed gets noticed.

    Each removal happens under the lease itself, and a busy one is skipped. A
    lease file another process holds *is* its mutual exclusion: unlinking it
    would leave that process holding a lock on an unreachable inode while the
    next run creates a fresh file and locks that one too, and both would
    believe they owned the prefix.
    """
    root = parent_dir(config)
    if not root.is_dir():
        return []
    removed: list[Path] = []
    for entry in sorted(root.iterdir()):
        if entry.is_dir():
            continue
        identity = _identity_of(config, entry.name)
        if identity is None or (root / identity).exists():
            continue
        try:
            with lease(config, root / identity):
                entry.unlink(missing_ok=True)
                removed.append(entry)
        except (PrefixBusy, GateError):
            continue
    return removed
