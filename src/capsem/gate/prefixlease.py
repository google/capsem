"""Cross-process ownership for deterministic private source directories."""

from __future__ import annotations

import fcntl
import os
from contextlib import contextmanager
from pathlib import Path

from .config import GateConfig
from .errors import GateError, PrefixBusy


def parent_dir(config: GateConfig) -> Path:
    return Path(config.prefix.parent).expanduser()


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
