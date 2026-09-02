"""Read-only filesystem inventory for policy-owned cache stages."""

from __future__ import annotations

import fnmatch
import os
import stat
import time
from pathlib import Path

from .contract import PruneStrategy
from .inventorymodels import RetentionInventory
from .leases import active_path
from .models import CacheEntry, CacheInventory, CachePolicy, StageInventory
from .paths import CachePaths


def _lease_active(stage_path: Path, template: str | None, key: str) -> bool:
    if template is None:
        return False
    lease = stage_path / template.format(key=key)
    if not lease.is_file() or lease.is_symlink():
        return False
    return active_path(lease)


def _entry_size(path: Path, allocated_seen: set[tuple[int, int]]) -> tuple[int, int]:
    logical = 0
    allocated = 0
    stack = [path]
    while stack:
        current = stack.pop()
        metadata = current.lstat()
        mode = metadata.st_mode
        if stat.S_ISLNK(mode):
            continue
        inode = (metadata.st_dev, metadata.st_ino)
        if stat.S_ISDIR(mode):
            with os.scandir(current) as children:
                stack.extend(Path(child.path) for child in children)
            continue
        if stat.S_ISREG(mode):
            logical += metadata.st_size
            if inode not in allocated_seen:
                allocated_seen.add(inode)
                allocated += getattr(metadata, "st_blocks", 0) * 512
    return logical, allocated


def _stage_inventory(
    stage_id: str,
    paths: CachePaths,
    policy: CachePolicy,
    allocated_seen: set[tuple[int, int]],
) -> StageInventory:
    stage_policy = policy.stages[stage_id]
    stage_root = paths.stage(stage_id)
    stage_path = stage_root / stage_policy.entry_root
    referenced = _referenced_keys(paths, stage_policy.selector_globs, stage_path)
    entries: list[CacheEntry] = []
    unmanaged_logical = 0
    unmanaged_allocated = 0
    if stage_path.is_dir():
        for child in sorted(stage_path.iterdir(), key=lambda item: item.name):
            logical, allocated = _entry_size(child, allocated_seen)
            stat = child.lstat()
            managed = any(
                fnmatch.fnmatchcase(child.name, pattern) for pattern in stage_policy.managed_globs
            )
            entries.append(
                CacheEntry(
                    key=child.name,
                    relative_path=Path(child.name),
                    logical_bytes=logical,
                    allocated_bytes=allocated,
                    created_ns=stat.st_ctime_ns,
                    last_used_ns=stat.st_atime_ns,
                    managed=managed,
                    protected=managed
                    and (
                        child.name in referenced
                        or _lease_active(stage_path, stage_policy.lease_template, child.name)
                    ),
                )
            )
    if stage_policy.entry_root != Path(".") and stage_root.is_dir():
        for child in stage_root.iterdir():
            if child == stage_path:
                continue
            logical, allocated = _entry_size(child, allocated_seen)
            unmanaged_logical += logical
            unmanaged_allocated += allocated
    return StageInventory(
        stage_id=stage_id,
        path=stage_path,
        logical_bytes=sum(entry.logical_bytes for entry in entries) + unmanaged_logical,
        allocated_bytes=sum(entry.allocated_bytes for entry in entries) + unmanaged_allocated,
        protected_bytes=sum(entry.logical_bytes for entry in entries if entry.protected),
        entries=tuple(entries),
    )


def _referenced_keys(
    paths: CachePaths, selector_globs: tuple[str, ...], entry_root: Path
) -> frozenset[str]:
    """Resolve configured selectors to the top-level cache generations they pin."""
    protected = set()
    resolved_root = entry_root.resolve()
    for pattern in selector_globs:
        for selector in paths.root.glob(pattern):
            if not selector.is_symlink():
                continue
            try:
                relative = selector.resolve().relative_to(resolved_root)
            except (OSError, ValueError):
                continue
            if relative.parts:
                protected.add(relative.parts[0])
    return frozenset(protected)


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
    return CacheInventory(
        root=paths.root,
        generated_ns=time.time_ns() if now_ns is None else now_ns,
        logical_bytes=sum(stage.logical_bytes for stage in stages)
        + sum(entry.logical_bytes for entry in unclassified),
        allocated_bytes=sum(stage.allocated_bytes for stage in stages)
        + sum(entry.allocated_bytes for entry in unclassified),
        stages=stages,
        unclassified=unclassified,
    )


def select_inventory(inventory: CacheInventory, stage_id: str) -> CacheInventory:
    """Return one disk owner while preserving the typed inventory shape."""
    if stage_id == "all":
        return inventory
    selected = tuple(stage for stage in inventory.stages if stage.stage_id == stage_id)
    if not selected:
        raise ValueError(f"unknown disk cache {stage_id!r}")
    return inventory.model_copy(
        update={
            "logical_bytes": sum(stage.logical_bytes for stage in selected),
            "allocated_bytes": sum(stage.allocated_bytes for stage in selected),
            "stages": selected,
            "unclassified": (),
        }
    )


def scan_retention_inventory(
    paths: CachePaths, policy: CachePolicy, *, now_ns: int | None = None
) -> RetentionInventory:
    """Scan only stages whose configured retention policy permits deletion."""
    allocated_seen: set[tuple[int, int]] = set()
    stage_ids = sorted(
        stage_id
        for stage_id, stage in policy.stages.items()
        if stage.prune_strategy is not PruneStrategy.NONE
    )

    stages = tuple(
        _stage_inventory(stage_id, paths, policy, allocated_seen) for stage_id in stage_ids
    )
    return RetentionInventory(
        root=paths.root,
        generated_ns=time.time_ns() if now_ns is None else now_ns,
        stages=stages,
    )
