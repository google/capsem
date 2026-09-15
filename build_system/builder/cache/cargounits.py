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
"""

from __future__ import annotations

import re
from pathlib import Path

from .measure import measure
from .models import CacheEntry

#: Scanned in this order; the first member of a unit is its fingerprint.
UNIT_DIRECTORIES = (".fingerprint", "build", "deps", "examples", "incremental")
_UNIT_HASH = re.compile(r"-([0-9a-f]{16})(?=[.-]|$)")


def unit_entries(
    stage_root: Path,
    target_roots: tuple[Path, ...],
    allocated_seen: set[tuple[int, int]],
    *,
    protected: bool,
) -> tuple[tuple[CacheEntry, ...], frozenset[Path]]:
    """Every unit under the target roots, and the paths they account for."""
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
                )
            )
    return tuple(entries), frozenset(accounted)


def unaccounted_size(
    root: Path, accounted: frozenset[Path], allocated_seen: set[tuple[int, int]]
) -> tuple[int, int]:
    """Bytes under `root` outside every unit, each counted once."""
    if root in accounted or root.is_symlink() or not root.exists():
        return 0, 0
    if root.is_dir() and any(root in path.parents for path in accounted):
        logical = allocated = 0
        for child in root.iterdir():
            child_logical, child_allocated = unaccounted_size(child, accounted, allocated_seen)
            logical += child_logical
            allocated += child_allocated
        return logical, allocated
    measured = measure(root, allocated_seen)
    return measured.logical_bytes, measured.allocated_bytes
