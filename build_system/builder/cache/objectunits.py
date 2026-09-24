"""The content-addressed object store, retained one receipt generation at a time.

`objects/blake3/` holds immutable bytes; `objects/components/<name>/<id>.json`
receipts map an exact build input to the objects that are its output. Neither
directory is a generation on its own: removing a blob a receipt names turns a
cache hit into a build failure, and listing the store's children offers only
`blake3/` itself as a candidate. So a generation is one receipt together with
the objects no other receipt names. The receipt is listed first, so an
interrupted removal leaves a clean miss rather than a receipt promising bytes
that are gone.

An object two receipts share belongs to neither and outlives both; the first
prune after its last receipt goes finds it unreferenced. An unreferenced
object -- a canonicalized package or copied release view, whose bytes live on
in the hardlinked view -- is a generation of its own, carrying the write-only
view receipts filed under its digest.
"""

from __future__ import annotations

import json
import re
from collections import Counter
from pathlib import Path

from .measure import measure
from .models import CacheEntry

OBJECTS = Path("blake3")
RECEIPTS = Path("components")
VIEWS = Path("receipts/views")
_DIGEST = re.compile(r"^[0-9a-f]{64}$")


def _named_objects(receipt: Path) -> frozenset[str]:
    """Digests a component receipt names; an unreadable receipt names none.

    Parsing stays structural rather than importing the image package's model:
    retention only needs to know which bytes a receipt would restore, and a
    malformed receipt protects nothing, so it can be collected like any other.
    """
    try:
        files = json.loads(receipt.read_text(encoding="utf-8")).get("files")
    except (OSError, ValueError, AttributeError):
        return frozenset()
    if not isinstance(files, dict):
        return frozenset()
    return frozenset(
        reference["digest"]
        for reference in files.values()
        if isinstance(reference, dict)
        and isinstance(reference.get("digest"), str)
        and _DIGEST.fullmatch(reference["digest"])
    )


def _objects(stage_root: Path) -> dict[str, Path]:
    found: dict[str, Path] = {}
    root = stage_root / OBJECTS
    if root.is_symlink() or not root.is_dir():
        return found
    for shard in sorted(root.iterdir()):
        if shard.is_symlink() or not shard.is_dir():
            continue
        for payload in sorted(shard.iterdir()):
            if not payload.is_symlink() and payload.is_file() and _DIGEST.fullmatch(payload.name):
                found[payload.name] = payload
    return found


def _receipts(stage_root: Path) -> list[Path]:
    root = stage_root / RECEIPTS
    if root.is_symlink() or not root.is_dir():
        return []
    return sorted(
        receipt
        for receipt in root.glob("*/*.json")
        if not receipt.is_symlink() and receipt.is_file()
    )


def object_entries(
    stage_root: Path,
    allocated_seen: set[tuple[int, int]],
    *,
    protected: bool,
) -> tuple[tuple[CacheEntry, ...], frozenset[Path]]:
    """Every generation in the store, and the paths they account for."""
    objects = _objects(stage_root)
    named = {receipt: _named_objects(receipt) for receipt in _receipts(stage_root)}
    references = Counter(digest for digests in named.values() for digest in digests)
    entries: list[CacheEntry] = []
    accounted: set[Path] = set()

    def views(digest: str) -> list[Path]:
        filed = stage_root / VIEWS / digest
        return [filed] if filed.is_dir() and not filed.is_symlink() else []

    def add(key: str, members: list[Path]) -> None:
        measured = [measure(member, allocated_seen) for member in members]
        accounted.update(members)
        entries.append(
            CacheEntry(
                key=key,
                relative_path=members[0].relative_to(stage_root),
                member_paths=tuple(member.relative_to(stage_root) for member in members[1:]),
                logical_bytes=sum(item.logical_bytes for item in measured),
                allocated_bytes=sum(item.allocated_bytes for item in measured),
                created_ns=min(item.last_used_ns for item in measured),
                last_used_ns=max(item.last_used_ns for item in measured),
                protected=protected,
            )
        )

    for receipt, digests in named.items():
        owned = [
            objects[digest]
            for digest in sorted(digests)
            if references[digest] == 1 and digest in objects
        ]
        filed = [view for digest in sorted(digests) if references[digest] == 1
                 for view in views(digest)]
        key = receipt.relative_to(stage_root).with_suffix("").as_posix()
        add(key, [receipt, *owned, *filed])
    for digest, payload in objects.items():
        if references[digest] == 0:
            add(f"{OBJECTS.as_posix()}/{digest}", [payload, *views(digest)])
    views_root = stage_root / VIEWS
    if views_root.is_dir() and not views_root.is_symlink():
        for filed in sorted(views_root.iterdir()):
            if filed.name not in objects and filed.is_dir() and not filed.is_symlink():
                add(f"{VIEWS.as_posix()}/{filed.name}", [filed])
    return tuple(entries), frozenset(accounted)
