"""Native process observations, with optional extended memory detail."""

import contextlib
from dataclasses import dataclass
from decimal import Decimal
from pathlib import Path

import psutil

from .process_cpu import process_cpu_seconds


@dataclass(frozen=True)
class ProcessUsage:
    pid: int
    started_at: float
    name: str
    cpu_seconds: Decimal
    rss_bytes: int
    pss_bytes: int | None
    uss_bytes: int | None
    fds: int
    rss_anon_bytes: int | None = None
    rss_file_bytes: int | None = None
    rss_shmem_bytes: int | None = None


@dataclass(frozen=True)
class TreeSnapshot:
    processes: dict[tuple[int, float], ProcessUsage]


def _linux_rss_breakdown(pid: int) -> tuple[int, int, int] | None:
    """Read Linux RSS categories without walking every memory mapping."""
    try:
        status = (Path(psutil.PROCFS_PATH) / str(pid) / "status").read_text(encoding="utf-8")
    except (AttributeError, OSError):
        return None
    values = {}
    for line in status.splitlines():
        name, separator, raw = line.partition(":")
        if separator and name in {"RssAnon", "RssFile", "RssShmem"}:
            fields = raw.split()
            if not fields:
                return None
            values[name] = int(fields[0]) * 1024
    if values.keys() != {"RssAnon", "RssFile", "RssShmem"}:
        return None
    return values["RssAnon"], values["RssFile"], values["RssShmem"]


def process_tree_snapshot(root: psutil.Process) -> TreeSnapshot:
    """Keep readable CPU/RSS/FD data when extended memory is unavailable.

    A process disappearing is normal during teardown. Permission failures on
    required counters or descendant enumeration invalidate accounting instead
    of silently undercounting the tree. macOS commonly denies full memory
    inspection of signed processes while exposing the basic counters.
    """
    found = {}
    with contextlib.suppress(psutil.NoSuchProcess):
        processes = [root, *root.children(recursive=True)]
        for process in processes:
            with contextlib.suppress(psutil.NoSuchProcess):
                with process.oneshot():
                    try:
                        memory = process.memory_full_info()
                    except (psutil.AccessDenied, NotImplementedError):
                        memory = process.memory_info()
                    rss_breakdown = _linux_rss_breakdown(process.pid)
                    usage = ProcessUsage(
                        pid=process.pid,
                        started_at=process.create_time(),
                        name=process.name(),
                        cpu_seconds=process_cpu_seconds(process),
                        rss_bytes=memory.rss,
                        pss_bytes=getattr(memory, "pss", None),
                        uss_bytes=getattr(memory, "uss", None),
                        fds=process.num_fds(),
                        rss_anon_bytes=None if rss_breakdown is None else rss_breakdown[0],
                        rss_file_bytes=None if rss_breakdown is None else rss_breakdown[1],
                        rss_shmem_bytes=None if rss_breakdown is None else rss_breakdown[2],
                    )
                found[(usage.pid, usage.started_at)] = usage
    return TreeSnapshot(found)
