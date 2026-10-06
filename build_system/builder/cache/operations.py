"""The sole filesystem mutation boundary for cache retention."""

from __future__ import annotations

import json
import os
import shutil
import stat
import time
from collections.abc import Iterator
from contextlib import ExitStack
from pathlib import Path

from pydantic import ValidationError

from .leases import exclusive_path, mutation_locks, release_path
from .measure import measure
from .models import AdmissionEvent, ApplyResult, PruneAction, PrunePlan
from .paths import CachePaths

JOURNAL_PATH = Path("state/events/cache.jsonl")


def _contained(root: Path, target: Path) -> Path:
    absolute_root = root.absolute()
    absolute_target = target.absolute()
    if absolute_target == absolute_root or absolute_root not in absolute_target.parents:
        raise ValueError(f"refusing cache target outside cache root: {target}")
    return absolute_target


def _make_owner_removable(root: Path) -> None:
    """Give the owner back every directory under ``root``, never following links.

    A test that makes a directory unwritable and dies before restoring it
    leaves a tree no plain ``rmtree`` can delete, and one such leftover used to
    fail every later prune. The cache owns the tree, so it takes the owner's
    bits back, top-down so an unreadable directory is opened before it is
    walked. A symlink is removed as a link; its target is never touched.
    """
    owner = stat.S_IRWXU
    root.chmod(root.lstat().st_mode | owner)
    for directory, subdirectories, _files in os.walk(root):
        for name in subdirectories:
            child = Path(directory) / name
            if not child.is_symlink():
                child.chmod(child.lstat().st_mode | owner)


def _remove(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    try:
        shutil.rmtree(path)
    except PermissionError:
        _make_owner_removable(path)
        shutil.rmtree(path)


def _unlocked_targets(target: Path, locks: tuple[Path, ...]) -> Iterator[Path]:
    """Cold clean keeps native lock inodes so a producer cannot replace them."""
    if target in locks:
        return
    if target.is_dir() and not target.is_symlink() and any(target in lock.parents for lock in locks):
        for child in sorted(target.iterdir()):
            yield from _unlocked_targets(child, locks)
    else:
        yield target


def apply_prune(paths: CachePaths, plan: PrunePlan, *, reason: str) -> ApplyResult:
    """Apply one reviewed plan and append its exact outcome to the journal."""
    if not reason.strip():
        raise ValueError("cache mutation reason must be non-empty")
    targets = tuple(paths.contained_entry(action.stage_id, action.path) for action in plan.actions)
    generations: dict[tuple[str, str], list[Path]] = {}
    for action, target in zip(plan.actions, targets, strict=True):
        generations.setdefault((action.stage_id, action.key), []).append(target)
    removed: list[Path] = []
    missing: list[Path] = []
    busy: list[Path] = []
    with mutation_locks(paths, (action.stage_id for action in plan.actions)) as locks:
        for (stage_id, key), selected in generations.items():
            policy = paths.policy.stages[stage_id]
            if policy.protect_hardlinks and any(_linked_file(path) for path in selected):
                busy.extend(selected)
                continue
            template = policy.lease_template
            root = paths.stage(stage_id)
            lease = None if template is None or not root.is_dir() else root / template.format(key=key)
            with ExitStack() as stack:
                if lease is not None and not stack.enter_context(exclusive_path(lease)):
                    busy.extend(selected)
                    continue
                for target in (*selected, *(() if lease is None else (lease,))):
                    for unlocked in _unlocked_targets(target, locks):
                        if unlocked.exists() or unlocked.is_symlink():
                            _remove(unlocked)
                            removed.append(unlocked)
                        elif unlocked != lease:
                            missing.append(unlocked)
    journal = paths.root / JOURNAL_PATH
    journal.parent.mkdir(parents=True, exist_ok=True)
    event = {
        "version": 1,
        "timestamp_ns": time.time_ns(),
        "plan_generated_ns": plan.generated_ns,
        "reason": reason,
        "removed": [str(path) for path in removed],
        "missing": [str(path) for path in missing],
        "busy": [str(path) for path in busy],
        "reclaim_bytes": plan.reclaim_bytes,
    }
    with journal.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(event, sort_keys=True) + "\n")
    return ApplyResult(
        removed=tuple(removed), missing=tuple(missing), busy=tuple(busy), journal=journal
    )


def _linked_file(path: Path) -> bool:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return False
    return stat.S_ISREG(metadata.st_mode) and metadata.st_nlink > 1


def reclaim_generation(paths: CachePaths, stage_id: str, key: str, *, reason: str) -> ApplyResult:
    """End one generation this process leased: release the lease, then remove
    the generation and its lease through the same guarded, journaled path a
    prune takes. A concurrent prune that wins the lease first does the same
    removal; whichever loses sees it busy or already gone."""
    stage = paths.policy.stages[stage_id]
    if stage.lease_template is None:
        raise ValueError(f"cache stage {stage_id!r} has no generation lease")
    root = paths.stage(stage_id)
    generation = root / key
    release_path(root / stage.lease_template.format(key=key))
    logical = measure(generation, set()).logical_bytes if generation.exists() else 0
    plan = PrunePlan(
        generated_ns=time.time_ns(),
        reclaim_bytes=logical,
        actions=(PruneAction(
            stage_id=stage_id, key=key, path=generation, logical_bytes=logical, reason=reason,
        ),),
        violations=(),
    )
    return apply_prune(paths, plan, reason=reason)


def record_admission_event(root: Path, state_path: Path, event: AdmissionEvent) -> Path:
    """Append one strict admission event beneath cache state."""
    target = _contained(root, root / state_path)
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("a", encoding="utf-8") as stream:
        stream.write(event.model_dump_json() + "\n")
    return target


def last_admission_event(root: Path, state_path: Path) -> AdmissionEvent | None:
    """Read the newest admission event; malformed state fails closed."""
    target = _contained(root, root / state_path)
    if not target.exists():
        return None
    lines = [line for line in target.read_text(encoding="utf-8").splitlines() if line]
    if not lines:
        return None
    try:
        return AdmissionEvent.model_validate_json(lines[-1])
    except ValidationError as error:
        raise ValueError(f"invalid cache admission state at {target}") from error
