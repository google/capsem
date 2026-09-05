"""The sole filesystem mutation boundary for cache retention."""

from __future__ import annotations

import json
import shutil
import time
from pathlib import Path

from pydantic import ValidationError

from .models import AdmissionEvent, ApplyResult, PrunePlan
from .paths import CachePaths

JOURNAL_PATH = Path("state/events/cache.jsonl")


def _contained(root: Path, target: Path) -> Path:
    absolute_root = root.absolute()
    absolute_target = target.absolute()
    if absolute_target == absolute_root or absolute_root not in absolute_target.parents:
        raise ValueError(f"refusing cache target outside cache root: {target}")
    return absolute_target


def _remove(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink()
    else:
        shutil.rmtree(path)


def apply_prune(paths: CachePaths, plan: PrunePlan, *, reason: str) -> ApplyResult:
    """Apply one reviewed plan and append its exact outcome to the journal."""
    if not reason.strip():
        raise ValueError("cache mutation reason must be non-empty")
    targets = tuple(paths.contained_entry(action.stage_id, action.path) for action in plan.actions)
    removed: list[Path] = []
    missing: list[Path] = []
    for target in targets:
        if target.exists() or target.is_symlink():
            _remove(target)
            removed.append(target)
        else:
            missing.append(target)
    journal = paths.root / JOURNAL_PATH
    journal.parent.mkdir(parents=True, exist_ok=True)
    event = {
        "version": 1,
        "timestamp_ns": time.time_ns(),
        "plan_generated_ns": plan.generated_ns,
        "reason": reason,
        "removed": [str(path) for path in removed],
        "missing": [str(path) for path in missing],
        "reclaim_bytes": plan.reclaim_bytes,
    }
    with journal.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(event, sort_keys=True) + "\n")
    return ApplyResult(removed=tuple(removed), missing=tuple(missing), journal=journal)


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
