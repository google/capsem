"""Read-only filesystem inventory for policy-owned cache stages."""

from __future__ import annotations

import time
from pathlib import Path

from .cargounits import cargo_inventory, unaccounted_size
from .contract import PruneStrategy
from .generations import directory_entries, entry_size
from .inventorymodels import RetentionInventory
from .leases import held_locks
from .models import CacheEntry, CacheInventory, CachePolicy, StageInventory
from .namespaces import foreign_entries, sweeps
from .objectunits import object_entries
from .paths import CachePaths


def _stage_inventory(
    stage_id: str,
    paths: CachePaths,
    policy: CachePolicy,
    allocated_seen: set[tuple[int, int]],
    *,
    retention: bool = False,
    checkout: Path | None = None,
) -> StageInventory:
    stage_policy = policy.stages[stage_id]
    stage_root = paths.stage(stage_id)
    if retention and stage_policy.cargo_target_roots:
        # The working set: the calling checkout and the cache authority.
        working_set = frozenset(root for root in (paths.repository_root, checkout) if root)
        return cargo_inventory(stage_id, stage_root, stage_policy, allocated_seen, working_set)
    if stage_policy.object_store:
        return _object_inventory(stage_id, stage_root, stage_policy, allocated_seen)
    entry_root = (
        stage_policy.retention_root
        if retention and stage_policy.retention_root
        else stage_policy.entry_root
    )
    stage_path = stage_root / entry_root
    if stage_path.is_symlink() or not stage_path.resolve().is_relative_to(stage_root.resolve()):
        raise ValueError(f"cache entry root escapes its stage: {stage_path}")
    referenced = _referenced_keys(paths, stage_policy.selector_globs, stage_path)
    entries: list[CacheEntry] = []
    unmanaged_logical = 0
    unmanaged_allocated = 0
    held = held_locks(stage_root, stage_policy.mutation_locks)
    busy = bool(held)
    sweep = sweeps(stage_policy)
    if stage_path.is_dir():
        entries = directory_entries(
            stage_path,
            stage_policy,
            allocated_seen,
            busy=busy,
            referenced=referenced,
            relative=Path(stage_path.name) if sweep else Path("."),
        )
    if sweep:
        entries += foreign_entries(stage_path, stage_policy, allocated_seen)
        stage_path = stage_path.parent
    if entry_root != Path(".") and stage_root.is_dir():
        # Account for siblings at every level, excluding the managed subtree
        # exactly once even when its entry_root is nested (Cargo incremental).
        ancestor = stage_root
        for part in entry_root.parts:
            selected = ancestor / part
            if ancestor.is_dir():
                for child in ancestor.iterdir():
                    if child != selected:
                        logical, allocated = entry_size(child, allocated_seen)
                        unmanaged_logical += logical
                        unmanaged_allocated += allocated
            ancestor = selected
    return StageInventory(
        stage_id=stage_id,
        path=stage_path,
        logical_bytes=sum(entry.logical_bytes for entry in entries) + unmanaged_logical,
        allocated_bytes=sum(entry.allocated_bytes for entry in entries) + unmanaged_allocated,
        protected_bytes=sum(entry.budget_bytes for entry in entries if entry.protected),
        entries=tuple(entries),
        held_locks=held,
    )


def _object_inventory(stage_id, stage_root: Path, stage_policy, allocated_seen) -> StageInventory:
    """An object store as its receipt generations, shared objects accounted beside them."""
    held = held_locks(stage_root, stage_policy.mutation_locks)
    busy = bool(held)
    entries, accounted = object_entries(stage_root, allocated_seen, protected=busy)
    # Shared objects are collected once no receipt names them: deferred, not
    # unselectable, so the store reports totals only.
    other_logical, other_allocated, _ = unaccounted_size(stage_root, accounted, allocated_seen)
    return StageInventory(
        stage_id=stage_id,
        path=stage_root,
        logical_bytes=sum(entry.logical_bytes for entry in entries) + other_logical,
        allocated_bytes=sum(entry.allocated_bytes for entry in entries) + other_allocated,
        protected_bytes=sum(entry.budget_bytes for entry in entries if entry.protected),
        entries=entries,
        held_locks=held,
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
        logical, allocated = entry_size(path, allocated_seen)
        try:
            stat = path.lstat()
        except FileNotFoundError:
            return  # Live producers may remove unclassified siblings too.
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
    paths: CachePaths,
    policy: CachePolicy,
    *,
    now_ns: int | None = None,
    retention: bool = False,
    stage_ids: frozenset[str] | None = None,
    checkout: Path | None = None,
) -> CacheInventory:
    """Scan configured leaves, or only named owners for a focused enforcement."""
    if stage_ids is not None:
        unknown = stage_ids.difference(policy.stages)
        if unknown:
            raise KeyError(f"unknown cache stages: {', '.join(sorted(unknown))}")
    allocated_seen: set[tuple[int, int]] = set()
    scan_order = sorted(
        (stage_id for stage_id in policy.stages if stage_ids is None or stage_id in stage_ids),
        key=lambda stage_id: (stage_id != "objects", stage_id),
    )
    by_id = {
        stage_id: _stage_inventory(
            stage_id, paths, policy, allocated_seen, retention=retention, checkout=checkout
        )
        for stage_id in scan_order
    }
    unclassified = (
        _unclassified_inventory(paths, policy, allocated_seen) if stage_ids is None else ()
    )
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
    paths: CachePaths,
    policy: CachePolicy,
    *,
    now_ns: int | None = None,
    checkout: Path | None = None,
) -> RetentionInventory:
    """Scan only stages whose configured retention policy permits deletion."""
    allocated_seen: set[tuple[int, int]] = set()
    stage_ids = sorted(
        stage_id
        for stage_id, stage in policy.stages.items()
        if stage.prune_strategy is not PruneStrategy.NONE
    )

    stages = tuple(
        _stage_inventory(stage_id, paths, policy, allocated_seen, retention=True, checkout=checkout)
        for stage_id in stage_ids
    )
    return RetentionInventory(
        root=paths.root,
        generated_ns=time.time_ns() if now_ns is None else now_ns,
        stages=stages,
    )
