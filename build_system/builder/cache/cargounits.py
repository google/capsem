"""Cargo target directories retained one compilation unit at a time.

Cargo names every artifact of a unit with the same 16-hex metadata hash:
its fingerprint, build-script output, libraries, dep-info, executables and
the signed copies `run_signed.sh` publishes beside them. A unit is therefore
the removable generation: taking all of it forces a clean rebuild, while
taking part of it could leave outputs a surviving fingerprint vouches for.
The fingerprint is listed first so an interrupted removal can only err
towards rebuilding.

Every gate prefix salts workspace units with its checkout path, so each
source state leaves a complete set behind; retention that knew only
`incremental/` reclaimed none of them (issue #205).

The salt is what keeps an older checkout from taking a newer one's unit as
fresh on mtime alone, so the copies cannot be shared on the pinned
toolchain (content-hash freshness is still `-Zchecksum-freshness`). They
are bounded by owner instead (issue #276): the workspace wrapper writes the
compiling checkout into each unit's fingerprint directory, a unit whose
checkout is gone is reclaimed outright, and under pressure other checkouts'
units go first, then shared or unrecorded ones, and the working set -- the
calling checkout and the cache authority -- last.
"""

from __future__ import annotations

import os
import re
from pathlib import Path

from .measure import measure
from .models import CacheEntry

#: Scanned in this order; the first member of a unit is its fingerprint.
UNIT_DIRECTORIES = (".fingerprint", "build", "deps", "examples", "incremental")
_UNIT_HASH = re.compile(r"-([0-9a-f]{16})(?=[.-]|$)")
#: A symlink to the compiling checkout, written by `rustc-workspace-wrapper.sh`
#: beside Cargo's fingerprint. A link, not a file: reading it moves no atime,
#: so it never disturbs the LRU clock, and a measure never counts it.
OWNER_FILE = "capsem-owner"
#: Eviction tiers, lowest first: another checkout's, shared or unrecorded,
#: then the working set.
OTHER_CHECKOUT, SHARED, WORKING_SET = 0, 1, 2


def _ownership(fingerprint: Path, working_set: frozenset[Path]) -> tuple[bool, int]:
    """Whether a unit's checkout is gone, and its eviction tier."""
    try:
        owner = Path(os.readlink(fingerprint / OWNER_FILE))
    except (FileNotFoundError, NotADirectoryError, OSError):
        return False, SHARED
    if not owner.is_absolute():
        return False, SHARED
    if not owner.exists():
        return True, OTHER_CHECKOUT
    return False, WORKING_SET if owner.resolve() in working_set else OTHER_CHECKOUT


def unit_entries(
    stage_root: Path,
    target_roots: tuple[Path, ...],
    allocated_seen: set[tuple[int, int]],
    *,
    protected: bool,
    working_set: frozenset[Path] = frozenset(),
) -> tuple[tuple[CacheEntry, ...], frozenset[Path]]:
    """Every unit under the target roots, and the paths they account for."""
    working_set = frozenset(path.resolve() for path in working_set)
    entries: list[CacheEntry] = []
    accounted: set[Path] = set()
    for target_root in target_roots:
        groups: dict[str, list[Path]] = {}
        for directory in UNIT_DIRECTORIES:
            parent = stage_root / target_root / directory
            if parent.is_symlink() or not parent.is_dir():
                continue
            for child in sorted(parent.iterdir(), key=lambda item: item.name):
                found = _UNIT_HASH.search(child.name)
                # Unhashed pieces, such as incremental sessions, stand alone.
                unit = found.group(1) if found else f"{directory}/{child.name}"
                groups.setdefault(f"{target_root.as_posix()}/{unit}", []).append(child)
        for key, members in groups.items():
            measured = [measure(member, allocated_seen) for member in members]
            relative = [member.relative_to(stage_root) for member in members]
            accounted.update(members)
            orphaned, rank = _ownership(members[0], working_set)
            entries.append(
                CacheEntry(
                    key=key,
                    relative_path=relative[0],
                    member_paths=tuple(relative[1:]),
                    logical_bytes=sum(item.logical_bytes for item in measured),
                    allocated_bytes=sum(item.allocated_bytes for item in measured),
                    created_ns=min(item.last_used_ns for item in measured),
                    last_used_ns=max(item.last_used_ns for item in measured),
                    protected=protected,
                    orphaned=orphaned,
                    retain_rank=rank,
                )
            )
    return tuple(entries), frozenset(accounted)


def unaccounted_size(
    root: Path, accounted: frozenset[Path], allocated_seen: set[tuple[int, int]]
) -> tuple[int, int]:
    """Bytes under `root` outside every unit, each counted once."""
    ancestors: set[Path] = set()
    for member in accounted:
        parent = member.parent
        while parent != root and parent != parent.parent:
            ancestors.add(parent)
            parent = parent.parent
        if parent == root:
            ancestors.add(root)

    def visit(path: Path) -> tuple[int, int]:
        if path in accounted or path.is_symlink() or not path.exists():
            return 0, 0
        if path.is_dir() and path in ancestors:
            logical = allocated = 0
            for child in path.iterdir():
                child_logical, child_allocated = visit(child)
                logical += child_logical
                allocated += child_allocated
            return logical, allocated
        measured = measure(path, allocated_seen)
        return measured.logical_bytes, measured.allocated_bytes

    return visit(root)
