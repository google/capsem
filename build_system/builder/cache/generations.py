"""The managed generations, leases, and unmanaged siblings of one stage directory."""

from __future__ import annotations

import fnmatch
from pathlib import Path

from .leases import active_path, lease_key
from .measure import measure
from .models import CacheEntry


def managed(stage_policy, name: str) -> bool:
    return any(fnmatch.fnmatchcase(name, pattern) for pattern in stage_policy.managed_globs)


def lease_active(directory: Path, template: str | None, key: str) -> bool:
    if template is None:
        return False
    lease = directory / template.format(key=key)
    if not lease.is_file() or lease.is_symlink():
        return False
    return active_path(lease)


def _lease_files(template: str | None, children: list[Path], stage_policy) -> dict[str, str]:
    """Map each lease file's name to the generation key it holds."""
    if template is None:
        return {}
    found = {}
    for child in children:
        key = lease_key(template, child.name)
        if key is not None and not managed(stage_policy, child.name) and not child.is_symlink() \
                and child.is_file():
            found[child.name] = key
    return found


def entry_size(path: Path, allocated_seen: set[tuple[int, int]]) -> tuple[int, int]:
    measured = measure(path, allocated_seen)
    return measured.logical_bytes, measured.allocated_bytes


def directory_entries(
    directory: Path,
    stage_policy,
    allocated_seen: set[tuple[int, int]],
    *,
    busy: bool,
    referenced: frozenset[str],
    relative: Path = Path("."),
    key_prefix: str = "",
) -> list[CacheEntry]:
    """Every child of `directory` as an entry: a managed generation with its
    lease, a lease whose generation is gone, or an unmanaged sibling.

    `relative` places the children beneath the inventory's stage path and
    `key_prefix` qualifies their keys; leases are always looked up beside them.
    """
    template = stage_policy.lease_template
    children = sorted(directory.iterdir(), key=lambda item: item.name)
    names = {child.name for child in children}
    managed_names = {name for name in names if managed(stage_policy, name)}
    leases = _lease_files(template, children, stage_policy)
    entries: list[CacheEntry] = []
    for child in children:
        key = leases.get(child.name)
        if key is not None and key in managed_names:
            continue  # removed and accounted with its generation
        if key in names:
            key = None  # the lease of an unmanaged sibling stays unmanaged
        logical, allocated = entry_size(child, allocated_seen)
        stat = child.lstat()
        lease_only = key is not None
        is_managed = lease_only or child.name in managed_names
        members: tuple[Path, ...] = ()
        if is_managed and not lease_only and template is not None:
            lease = template.format(key=child.name)
            if lease in leases:
                members = (relative / lease,)
                lease_logical, lease_allocated = entry_size(directory / lease, allocated_seen)
                logical += lease_logical
                allocated += lease_allocated
        generation = key or child.name
        entries.append(
            CacheEntry(
                key=key_prefix + generation,
                relative_path=relative / child.name,
                member_paths=members,
                logical_bytes=logical,
                allocated_bytes=allocated,
                created_ns=stat.st_ctime_ns,
                last_used_ns=stat.st_atime_ns,
                managed=is_managed,
                lease_only=lease_only,
                protected=is_managed
                and (
                    busy or generation in referenced
                    or lease_active(directory, template, generation)
                ),
            )
        )
    return entries
