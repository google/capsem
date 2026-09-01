"""Bounded failure-evidence capture owned by the cache library."""

from __future__ import annotations

import re
import shutil
import time
from pathlib import Path

from .failuremodels import (
    CollectedEvidence,
    CollectionOutcome,
    FailureEvidenceManifest,
)
from .models import CachePolicy
from .paths import CachePaths
from .runtimeexec import CommandRunner, execute
from .runtimeinventory import scan_runtimes


def _size(path: Path) -> int:
    total = 0
    for candidate in path.rglob("*"):
        try:
            if candidate.is_file():
                total += candidate.stat().st_size
        except OSError:
            continue
    return total


def _rotate(root: Path, policy, *, now: float) -> None:
    try:
        directories = sorted(
            (path for path in root.iterdir() if path.is_dir()),
            key=lambda path: (path.stat().st_mtime, path.name),
        )
    except OSError:
        return
    protected = set(directories[-policy.minimum_count :]) if policy.minimum_count else set()
    cutoff = now - policy.maximum_age_hours * 3600
    stale = list(directories[: -policy.maximum_count])
    stale.extend(
        path for path in directories if path not in protected and path.stat().st_mtime < cutoff
    )
    for path in dict.fromkeys(stale):
        shutil.rmtree(path, ignore_errors=True)
    remaining = tuple(path for path in directories if path.exists())
    sizes = {path: _size(path) for path in remaining}
    total = sum(sizes.values())
    remaining_count = len(remaining)
    for path in remaining:
        if total <= policy.maximum_bytes or remaining_count <= policy.minimum_count:
            break
        if path in protected:
            continue
        shutil.rmtree(path, ignore_errors=True)
        total -= sizes[path]
        remaining_count -= 1


def _copy(source: Path, destination: Path, maximum_bytes: int) -> CollectionOutcome:
    try:
        destination.parent.mkdir(parents=True, exist_ok=True)
        size = source.stat().st_size
        if size > maximum_bytes:
            with source.open("rb") as input_stream, destination.open("wb") as output_stream:
                input_stream.seek(-maximum_bytes, 2)
                shutil.copyfileobj(input_stream, output_stream)
            return CollectionOutcome.TRUNCATED
        shutil.copy2(source, destination)
        return CollectionOutcome.COPIED
    except OSError:
        return CollectionOutcome.UNREADABLE


def capture_failure(
    paths: CachePaths,
    policy: CachePolicy,
    *,
    label: str,
    run_id: str | None = None,
    source_commit: str | None = None,
    offline: bool = False,
    runner: CommandRunner = execute,
    now_ns: int | None = None,
) -> Path:
    """Capture typed runtime state and configured small files, then rotate."""
    if policy.control is None:
        raise ValueError("cache policy has no failure evidence control")
    settings = policy.control.failure_artifacts
    safe_label = re.sub(r"[^A-Za-z0-9_.-]+", "-", label).strip("-") or "candidate"
    created_ns = time.time_ns() if now_ns is None else now_ns
    manifest = FailureEvidenceManifest(
        created_ns=created_ns,
        label=safe_label,
        run_id=run_id,
        source_commit=source_commit,
        files=(),
    )
    stamp = time.strftime("%Y%m%d-%H%M%S", time.gmtime(created_ns / 1_000_000_000))
    root = paths.stage(settings.stage)
    destination = root / f"{stamp}-cache-{safe_label}"
    destination.mkdir(parents=True, exist_ok=False)
    snapshot = scan_runtimes(policy, runner=runner, now_ns=created_ns, offline=offline)
    (destination / "runtime-snapshot.json").write_text(
        snapshot.model_dump_json(indent=2) + "\n", encoding="utf-8"
    )
    collected = []
    for pattern in settings.source_patterns:
        matches = sorted(path for path in paths.root.glob(pattern) if path.is_file())
        if not matches:
            collected.append(
                CollectedEvidence(
                    source=paths.root / pattern,
                    destination=None,
                    outcome=CollectionOutcome.ABSENT,
                )
            )
        for source in matches:
            if source.name in settings.skip_names:
                continue
            relative = source.relative_to(paths.root)
            target = destination / "files" / relative
            outcome = _copy(source, target, settings.maximum_file_bytes)
            collected.append(CollectedEvidence(source=source, destination=target, outcome=outcome))
    manifest = manifest.model_copy(update={"files": tuple(collected)})
    (destination / "manifest.json").write_text(
        manifest.model_dump_json(indent=2) + "\n", encoding="utf-8"
    )
    _rotate(root, settings, now=created_ns / 1_000_000_000)
    return destination
