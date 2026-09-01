"""Read-only filesystem inventory for policy-owned cache stages."""

from __future__ import annotations

import os
import shutil
import time
from pathlib import Path

from .models import CacheEntry, CacheInventory, CachePolicy, StageInventory
from .paths import CachePaths


def _entry_size(path: Path, allocated_seen: set[tuple[int, int]]) -> tuple[int, int]:
    logical = 0
    allocated = 0
    stack = [path]
    while stack:
        current = stack.pop()
        stat = current.lstat()
        if current.is_symlink():
            continue
        inode = (stat.st_dev, stat.st_ino)
        if current.is_dir():
            with os.scandir(current) as children:
                stack.extend(Path(child.path) for child in children)
            continue
        if current.is_file():
            logical += stat.st_size
            if inode not in allocated_seen:
                allocated_seen.add(inode)
                allocated += getattr(stat, "st_blocks", 0) * 512
    return logical, allocated


def _stage_inventory(
    stage_id: str,
    paths: CachePaths,
    policy: CachePolicy,
    allocated_seen: set[tuple[int, int]],
) -> StageInventory:
    stage_policy = policy.stages[stage_id]
    stage_path = paths.stage(stage_id)
    entries: list[CacheEntry] = []
    if stage_path.is_dir():
        for child in sorted(stage_path.iterdir(), key=lambda item: item.name):
            logical, allocated = _entry_size(child, allocated_seen)
            stat = child.lstat()
            entries.append(
                CacheEntry(
                    key=child.name,
                    relative_path=Path(child.name),
                    logical_bytes=logical,
                    allocated_bytes=allocated,
                    created_ns=stat.st_ctime_ns,
                    last_used_ns=stat.st_atime_ns,
                )
            )
    return StageInventory(
        stage_id=stage_id,
        path=stage_path,
        external=stage_policy.external,
        logical_bytes=sum(entry.logical_bytes for entry in entries),
        allocated_bytes=sum(entry.allocated_bytes for entry in entries),
        protected_bytes=sum(entry.logical_bytes for entry in entries if entry.protected),
        entries=tuple(entries),
    )


def _unclassified_inventory(
    paths: CachePaths,
    policy: CachePolicy,
    allocated_seen: set[tuple[int, int]],
) -> tuple[CacheEntry, ...]:
    """Return minimal cache roots not owned by any configured stage."""
    stage_paths = tuple(stage.path for stage in policy.stages.values())
    entries: list[CacheEntry] = []

    def visit(path: Path, relative: Path) -> None:
        if relative in stage_paths:
            return
        if path.is_dir() and any(relative in stage.parents for stage in stage_paths):
            for child in sorted(path.iterdir(), key=lambda item: item.name):
                visit(child, relative / child.name)
            return
        logical, allocated = _entry_size(path, allocated_seen)
        stat = path.lstat()
        entries.append(
            CacheEntry(
                key=relative.as_posix(),
                relative_path=relative,
                logical_bytes=logical,
                allocated_bytes=allocated,
                created_ns=stat.st_ctime_ns,
                last_used_ns=stat.st_atime_ns,
            )
        )

    if paths.root.is_dir():
        for child in sorted(paths.root.iterdir(), key=lambda item: item.name):
            visit(child, Path(child.name))
    return tuple(entries)


def scan_inventory(
    paths: CachePaths, policy: CachePolicy, *, now_ns: int | None = None
) -> CacheInventory:
    """Scan configured leaves without creating cache directories or following links."""
    allocated_seen: set[tuple[int, int]] = set()
    scan_order = sorted(policy.stages, key=lambda stage_id: (stage_id != "objects", stage_id))
    by_id = {
        stage_id: _stage_inventory(stage_id, paths, policy, allocated_seen)
        for stage_id in scan_order
    }
    unclassified = _unclassified_inventory(paths, policy, allocated_seen)
    stages = tuple(by_id[stage_id] for stage_id in sorted(by_id))
    free = shutil.disk_usage(paths.root.parent).free
    return CacheInventory(
        root=paths.root,
        generated_ns=time.time_ns() if now_ns is None else now_ns,
        filesystem_free_bytes=free,
        logical_bytes=sum(stage.logical_bytes for stage in stages)
        + sum(entry.logical_bytes for entry in unclassified),
        allocated_bytes=sum(stage.allocated_bytes for stage in stages)
        + sum(entry.allocated_bytes for entry in unclassified),
        stages=stages,
        unclassified=unclassified,
    )
