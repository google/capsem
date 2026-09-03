"""High-resolution process CPU accounting for performance gates."""

from __future__ import annotations

from decimal import Decimal
from pathlib import Path
from typing import Protocol

NANOSECONDS_PER_SECOND = Decimal(1_000_000_000)


class CpuTimes(Protocol):
    @property
    def user(self) -> float: ...

    @property
    def system(self) -> float: ...


class ProcessLike(Protocol):
    @property
    def pid(self) -> int: ...

    def cpu_times(self) -> CpuTimes: ...


def process_cpu_seconds(
    process: ProcessLike,
    *,
    proc_root: Path = Path("/proc"),
) -> Decimal:
    """Read total process CPU with nanosecond precision when Linux exposes it.

    ``/proc/<pid>/schedstat`` covers only the main thread. Tokio does its work
    on worker threads, so Linux measurements must sum every task. Other
    platforms retain psutil's portable user-plus-system counter.
    """
    task_root = proc_root / str(process.pid) / "task"
    if task_root.is_dir():
        schedstats = tuple(task_root.glob("*/schedstat"))
        if schedstats:
            runtime_ns = sum(_runtime_nanoseconds(path) for path in schedstats)
            return Decimal(runtime_ns) / NANOSECONDS_PER_SECOND

    times = process.cpu_times()
    return Decimal(str(times.user)) + Decimal(str(times.system))


def _runtime_nanoseconds(path: Path) -> int:
    fields = path.read_text(encoding="utf-8").split()
    if not fields:
        raise ValueError(f"empty scheduler accounting file: {path}")
    return int(fields[0])
