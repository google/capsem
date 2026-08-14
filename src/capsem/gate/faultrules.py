"""What counts as a filesystem fault, separately from watching for one.

Split from `observation`, which was at the three-hundred-line ceiling the gate
holds itself to and could not absorb a two-line change. The seam is a real one:
this module answers "is this event wrong", and that is the half people come to
edit -- a new rule lands here without touching the machinery that produced the
event.

Every rule reports through `watch.fault`, so the watch stays the single owner
of where a fault goes and what a run does about it.
"""

from __future__ import annotations

import stat
from pathlib import Path
from typing import TYPE_CHECKING

from .faults import duplication_expected

#: The one fault reason a publishing run treats as fatal; `observing` reads it.
SOURCE_TREE = "source-tree"

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from .faults import Event
    from .observation import Watch


def judge(watch: Watch, event: Event) -> None:
    source = watch.is_source(event.path)

    if source and event.kind != "deleted":
        watch.fault(
            event, SOURCE_TREE, f"{event.kind} during the run; the gate qualifies this tree"
        )

    if not source and event.inode is not None and event.links and event.links > 1:
        origin = watch._source_inodes.get(event.inode)
        if origin is not None:
            watch.fault(
                event,
                "hardlinked-source",
                f"shares inode {event.inode} (nlink={event.links}) with checked-in "
                f"{origin.relative_to(watch._source_root)}, so a chmod here rewrites "
                "tracked source and no content digest will notice",
            )

    if event.mode is not None:
        history = watch._modes.setdefault(event.path, [])
        # Returning to any mode already seen -- not merely the one before
        # last -- is the flip-flop: 0644 -> 0000 -> 0644 is identical at
        # both ends and unreadable in the middle.
        if history and event.mode != history[-1] and event.mode in history[:-1]:
            watch.fault(
                event,
                "mode-flip-flop",
                f"{history[-1]:04o} -> {event.mode:04o}, back to a mode it already had; "
                "a concurrent reader sees the middle state and fails intermittently",
            )
        if not history or history[-1] != event.mode:
            history.append(event.mode)
        if source and event.mode & (stat.S_IWGRP | stat.S_IWOTH):
            watch.fault(
                event, "over-permission", f"mode {event.mode:04o} is writable beyond its owner"
            )

    exempt = duplication_expected(
        event.path,
        watch._source_root,
        (*watch._duplicate_exempt, *watch._source_replicas),
    )
    if event.digest is not None and not exempt:
        first = watch._digests.setdefault(event.digest, event.path)
        if first != event.path and event.inode is not None:
            watch.fault(event, "duplicate-content", f"identical bytes already at {first}")

    if event.attribution == "exact" and len(event.steps) >= 2 and not source:
        shared = set.intersection(
            *(set(watch._declared.get(step, frozenset())) for step in event.steps)
        )
        if not shared:
            watch.fault(
                event,
                "undeclared-contention",
                "two steps the scheduler ran together both touched this, and neither "
                "declares sharing it",
            )


def is_source_replica(watch: Watch, path: Path) -> bool:
    """Whether content under `path` is the declared frozen source copy."""
    return duplication_expected(path, watch._source_root, watch._source_replicas)
