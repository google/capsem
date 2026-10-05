"""The namespaces other cache authorities left in a shared external stage.

An external stage is namespaced by `sha256(authority)[:8]` (`CachePaths.stage`),
and prune once looked only at its own authority's namespace. One whose
authority was gone -- a deleted worktree, a test's temporary checkout -- was
never looked at again: ~350 of them held 2.4 GB on one machine (issue #272).

A leased ephemeral generation lives exactly as long as a process holds its
lease, whichever namespace it is in, so those stages are swept whole. Every
other namespace's runs and leases are entries keyed `<namespace>/<run>`, under
the same lease guard, and an empty namespace directory goes too. Only a real
directory named like a namespace is entered; a symlink, a pre-namespace
directory, or anything else at the stage root is never touched.
"""

from __future__ import annotations

import errno
import os
import re
from pathlib import Path

from .contract import PruneStrategy
from .generations import directory_entries, managed
from .models import CacheEntry
from .paths import CachePaths

NAMESPACE = re.compile(r"[0-9a-f]{8}")


def sweeps(stage_policy) -> bool:
    """Whether a stage's every namespace is this authority's to reclaim."""
    return (
        stage_policy.external
        and stage_policy.prune_strategy is PruneStrategy.EPHEMERAL
        and stage_policy.lease_template is not None
        and stage_policy.entry_root == Path(".")
        and stage_policy.retention_root is None
    )


def _real_namespace(root: Path, name: str) -> bool:
    directory = root / name
    return (
        NAMESPACE.fullmatch(name) is not None
        and not directory.is_symlink()
        and directory.is_dir()
        and directory.resolve() == root.resolve() / name
    )


def foreign_entries(
    own: Path, stage_policy, allocated_seen: set[tuple[int, int]]
) -> list[CacheEntry]:
    """Every other namespace's children, keyed and placed `<namespace>/<name>`
    beneath the stage root; an empty namespace is an entry of its own."""
    root = own.parent
    if root.is_symlink() or not root.is_dir():
        return []
    entries: list[CacheEntry] = []
    for child in sorted(root.iterdir(), key=lambda item: item.name):
        if child.name == own.name or not _real_namespace(root, child.name):
            continue
        found = directory_entries(
            child, stage_policy, allocated_seen, busy=False, referenced=frozenset(),
            relative=Path(child.name), key_prefix=f"{child.name}/",
        )
        if not found:
            stat = child.lstat()
            found = [CacheEntry(
                key=f"{child.name}/", relative_path=Path(child.name), logical_bytes=0,
                allocated_bytes=0, created_ns=stat.st_ctime_ns, last_used_ns=stat.st_atime_ns,
                lease_only=True,
            )]
        entries.extend(found)
    return entries


def _foreign(paths: CachePaths, stage_id: str, key: str) -> tuple[str, str] | None:
    """`(namespace, generation)` for a foreign key; generation is "" for the namespace."""
    if not sweeps(paths.policy.stages[stage_id]):
        return None
    namespace, slash, generation = key.partition("/")
    if not slash:
        return None
    if NAMESPACE.fullmatch(namespace) is None or namespace == paths.stage(stage_id).name:
        return None
    return namespace, generation


def lease_path(paths: CachePaths, stage_id: str, key: str) -> Path | None:
    """The lease that guards one planned generation, in whichever namespace."""
    template = paths.policy.stages[stage_id].lease_template
    root = paths.stage(stage_id)
    if template is None:
        return None
    found = _foreign(paths, stage_id, key)
    if found is None:
        return root / template.format(key=key) if root.is_dir() else None
    namespace, generation = found
    if not generation:
        return None  # a namespace is removed only while empty, never leased
    return root.parent / namespace / template.format(key=generation)


def contained(paths: CachePaths, stage_id: str, key: str, target: Path) -> Path:
    """Resolve one removable path: beneath this authority's namespace as ever,
    or exactly a foreign namespace or one of its direct children, matching
    the planned key's namespace, generation, and lease names."""
    found = _foreign(paths, stage_id, key)
    if found is None:
        return paths.contained_entry(stage_id, target)
    root = paths.stage(stage_id).parent.absolute()
    absolute = target.absolute()
    relative = absolute.relative_to(root).parts if absolute.is_relative_to(root) else ()
    namespace, generation = found
    stage_policy = paths.policy.stages[stage_id]
    template = stage_policy.lease_template
    names = {generation} if template is None else {generation, template.format(key=generation)}
    if (
        not relative
        or relative[0] != namespace
        or len(relative) != (2 if generation else 1)
        or (generation and (relative[1] not in names or not managed(stage_policy, generation)))
        or any(part in {"", ".", ".."} for part in relative)
        or not _real_namespace(root, namespace)
    ):
        raise ValueError(f"refusing target outside cache stage {stage_id!r}: {target}")
    return absolute


def namespace_of(paths: CachePaths, stage_id: str, key: str) -> Path | None:
    """The foreign namespace directory a planned key lives in, if any."""
    found = _foreign(paths, stage_id, key)
    return None if found is None else paths.stage(stage_id).parent.absolute() / found[0]


def remove_if_empty(directory: Path) -> bool:
    """Remove a namespace directory only while it is empty; never recursive.

    An owner that created it but has not yet created its lease retakes the
    directory (`leases.retain_path`)."""
    try:
        os.rmdir(directory)
    except OSError as error:
        if error.errno in {errno.ENOTEMPTY, errno.EEXIST, errno.ENOENT, errno.ENOTDIR}:
            return False
        raise
    return True
