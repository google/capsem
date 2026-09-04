"""Bound Capsem integration fixtures in operating-system temp directories."""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

from .cleanup_common import StageResult, remove_path

TMP_DIR_MAX_AGE_S = 60 * 60
TEST_TMP_BUDGET_GB = 24.0
TMP_DIR_PREFIXES = ("capsem-test-", "capsem-e2e-", "capsem-gw-", "capsem-install-")
LINUX_TEST_TMP_PARENT = Path("/var/tmp/capsem-tests")


def clean_tmp_fixtures(tmp_dir: Path, dry_run: bool, verbose: bool) -> StageResult:
    """Remove stale Capsem test fixture directories older than one hour."""
    start = time.monotonic()
    if not tmp_dir.is_dir():
        return StageResult("tmp", 0, time.monotonic() - start, str(tmp_dir))

    cutoff = time.time() - TMP_DIR_MAX_AGE_S
    removed = 0
    try:
        with os.scandir(tmp_dir) as iterator:
            entries = list(iterator)
    except OSError:
        return StageResult("tmp", 0, time.monotonic() - start, str(tmp_dir))

    for entry in entries:
        if not entry.is_dir(follow_symlinks=False):
            continue
        if not any(entry.name.startswith(prefix) for prefix in TMP_DIR_PREFIXES):
            continue
        try:
            modified = entry.stat(follow_symlinks=False).st_mtime
        except OSError:
            continue
        if modified >= cutoff:
            continue
        path = Path(entry.path)
        if verbose:
            print(f"  rm {path}")
        if remove_path(path, dry_run):
            removed += 1

    return StageResult("tmp", removed, time.monotonic() - start, str(tmp_dir))


def _test_tmp_budget_gb() -> float:
    raw = os.environ.get("CAPSEM_TEST_TMP_BUDGET_GB", "").strip()
    if not raw:
        return TEST_TMP_BUDGET_GB
    try:
        return max(float(raw), 0.0)
    except ValueError:
        return TEST_TMP_BUDGET_GB


def _tmp_fixture_entry(entry: os.DirEntry) -> bool:
    return entry.is_dir(follow_symlinks=False) and any(
        entry.name.startswith(prefix) for prefix in TMP_DIR_PREFIXES
    )


def _disk_usage_bytes(path: str) -> int:
    """Return actual allocated bytes for a path, with logical-size fallback."""
    try:
        metadata = os.lstat(path)
    except OSError:
        return 0
    if hasattr(metadata, "st_blocks"):
        return int(metadata.st_blocks) * 512
    return int(metadata.st_size)


def _entry_disk_usage_bytes(entry: os.DirEntry) -> int:
    """Return allocated bytes so sparse VM images do not exhaust the budget."""
    try:
        if entry.is_symlink():
            return 0
        total = _disk_usage_bytes(entry.path)
        if entry.is_dir(follow_symlinks=False):
            for root_dir, dirs, files in os.walk(entry.path, followlinks=False):
                for name in dirs:
                    total += _disk_usage_bytes(os.path.join(root_dir, name))
                for name in files:
                    total += _disk_usage_bytes(os.path.join(root_dir, name))
        return total
    except OSError:
        return 0


def _entry_size_bytes(entry: os.DirEntry) -> int:
    try:
        if entry.is_symlink():
            return 0
        if entry.is_file(follow_symlinks=False):
            return entry.stat(follow_symlinks=False).st_size
        if entry.is_dir(follow_symlinks=False):
            total = 0
            for root_dir, _dirs, files in os.walk(entry.path):
                for name in files:
                    try:
                        total += os.lstat(os.path.join(root_dir, name)).st_size
                    except OSError:
                        continue
            return total
    except OSError:
        return 0
    return 0


def _prune_to_size_budget(
    parent: Path,
    budget_bytes: int,
    entry_filter,
    dry_run: bool,
    verbose: bool,
    size_fn=_entry_size_bytes,
) -> int:
    """Remove oldest eligible entries until their total fits the budget."""
    if not parent.is_dir():
        return 0
    try:
        scored: list[tuple[float, int, Path]] = []
        total = 0
        with os.scandir(parent) as entries:
            for entry in entries:
                if not entry_filter(entry):
                    continue
                try:
                    modified = entry.stat(follow_symlinks=False).st_mtime
                except OSError:
                    continue
                size = size_fn(entry)
                scored.append((modified, size, Path(entry.path)))
                total += size
    except OSError:
        return 0
    if total <= budget_bytes:
        return 0
    scored.sort(key=lambda item: item[0])
    removed = 0
    for _modified, size, path in scored:
        if total <= budget_bytes:
            break
        if verbose:
            print(f"  rm {path} (size={size / 1024 / 1024:.0f} MB, over-budget)")
        if remove_path(path, dry_run):
            total -= size
            removed += 1
    return removed


def clean_tmp_fixtures_to_budget(
    tmp_dir: Path, dry_run: bool, verbose: bool
) -> StageResult:
    """Remove oldest recent Capsem fixtures until allocated use fits the budget."""
    start = time.monotonic()
    budget_gb = _test_tmp_budget_gb()
    if budget_gb <= 0:
        return StageResult("tmp-budget", 0, time.monotonic() - start, f"disabled {tmp_dir}")
    if not tmp_dir.is_dir():
        return StageResult("tmp-budget", 0, time.monotonic() - start, str(tmp_dir))

    removed = _prune_to_size_budget(
        tmp_dir,
        int(budget_gb * 1024**3),
        entry_filter=_tmp_fixture_entry,
        dry_run=dry_run,
        verbose=verbose,
        size_fn=_entry_disk_usage_bytes,
    )
    return StageResult(
        "tmp-budget",
        removed,
        time.monotonic() - start,
        f"budget={budget_gb:g}GB {tmp_dir}",
    )


def tmp_fixture_roots(primary: Path) -> list[Path]:
    """Return unique temp roots that can contain Capsem integration fixtures."""
    roots: list[Path] = []
    seen: set[Path] = set()

    def add(path: Path) -> None:
        normalized = path.expanduser().resolve(strict=False)
        if normalized not in seen:
            seen.add(normalized)
            roots.append(normalized)

    add(primary)
    configured = os.environ.get("CAPSEM_TEST_TMPDIR")
    if configured:
        add(Path(configured))
    if sys.platform.startswith("linux"):
        add(LINUX_TEST_TMP_PARENT)
    return roots
