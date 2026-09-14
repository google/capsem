"""One walk that sizes a cache path and dates its last use."""

from __future__ import annotations

import os
import stat
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Measure:
    logical_bytes: int
    allocated_bytes: int
    #: The latest access or modification of any file beneath the path.
    last_used_ns: int


def measure(path: Path, allocated_seen: set[tuple[int, int]]) -> Measure:
    """Size `path` without following links; hardlinked blocks count once."""
    logical = allocated = last_used = 0
    stack = [path]
    while stack:
        current = stack.pop()
        metadata = current.lstat()
        mode = metadata.st_mode
        if stat.S_ISLNK(mode):
            continue
        if stat.S_ISDIR(mode):
            with os.scandir(current) as children:
                stack.extend(Path(child.path) for child in children)
            continue
        if stat.S_ISREG(mode):
            logical += metadata.st_size
            last_used = max(last_used, metadata.st_atime_ns, metadata.st_mtime_ns)
            inode = (metadata.st_dev, metadata.st_ino)
            if inode not in allocated_seen:
                allocated_seen.add(inode)
                allocated += getattr(metadata, "st_blocks", 0) * 512
    return Measure(logical, allocated, last_used)
