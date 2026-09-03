"""Shared result and scoped removal primitives for host cleanup."""

from __future__ import annotations

import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass
class StageResult:
    """One cleanup stage's observable result."""

    name: str
    removed: int
    elapsed_s: float
    detail: str = ""
    bytes_before: int = 0
    bytes_after: int = 0

    @property
    def bytes_reclaimed(self) -> int:
        return max(0, self.bytes_before - self.bytes_after)


def remove_path(path: Path, dry_run: bool) -> bool:
    """Remove one resolved cleanup candidate, or only report it in dry-run mode."""
    if dry_run:
        return True
    try:
        if path.is_symlink() or path.is_file():
            path.unlink(missing_ok=True)
        elif path.is_dir():
            shutil.rmtree(path, ignore_errors=True)
        else:
            try:
                path.unlink()
            except FileNotFoundError:
                return False
        return True
    except OSError:
        return False


def allocated_size_bytes(path: Path) -> int | None:
    """Return allocated bytes reported by du, or None when unavailable."""
    if not path.is_dir():
        return None
    try:
        result = subprocess.run(
            ["du", "-sk", str(path)],
            capture_output=True,
            text=True,
            check=True,
            timeout=60,
        )
        return int(result.stdout.split()[0]) * 1024
    except (
        subprocess.TimeoutExpired,
        subprocess.CalledProcessError,
        FileNotFoundError,
        ValueError,
        IndexError,
    ):
        return None


def human_bytes(value: int) -> str:
    """Format an integral byte count for cleanup evidence."""
    if value >= 1024**3:
        return f"{value / 1024**3:.1f} GiB"
    if value >= 1024**2:
        return f"{value / 1024**2:.1f} MiB"
    return f"{value} B"
