"""Process-tree resource accounting for load benchmarks."""

from __future__ import annotations

import contextlib
import re
import threading
from dataclasses import dataclass
from decimal import Decimal

import psutil

from .process_cpu import process_cpu_seconds

SAMPLE_INTERVAL_SECONDS = 0.05
TRACKED_PROCESS_NAMES = frozenset(
    {
        "capsem_network",
        "capsem_process",
        "capsem_proxy",
        "capsem_router",
        "capsem_service",
    }
)


@dataclass(frozen=True)
class ProcessUsage:
    pid: int
    started_at: float
    name: str
    cpu_seconds: Decimal
    rss_bytes: int
    fds: int


@dataclass(frozen=True)
class TreeSnapshot:
    processes: dict[tuple[int, float], ProcessUsage]


def process_tree_snapshot(root: psutil.Process) -> TreeSnapshot:
    """Read one race-tolerant snapshot of a process and its descendants."""
    with contextlib.suppress(psutil.Error):
        processes = [root, *root.children(recursive=True)]
        found = {}
        for process in processes:
            with contextlib.suppress(psutil.Error, OSError):
                with process.oneshot():
                    usage = ProcessUsage(
                        pid=process.pid,
                        started_at=process.create_time(),
                        name=process.name(),
                        cpu_seconds=process_cpu_seconds(process),
                        rss_bytes=process.memory_info().rss,
                        fds=process.num_fds(),
                    )
                found[(usage.pid, usage.started_at)] = usage
        return TreeSnapshot(found)
    return TreeSnapshot({})


def _metric_name(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", name.lower()).strip("_")


class ProcessTreeSampler:
    """Track CPU deltas and peak resident resources without spawning tools."""

    def __init__(
        self,
        root: psutil.Process | None,
        *,
        interval_seconds: float = SAMPLE_INTERVAL_SECONDS,
    ) -> None:
        self._root = root
        self._interval_seconds = interval_seconds
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._observations = 0
        self._baseline: dict[str, tuple[int, int]] = {}
        self._peak: dict[str, tuple[int, int]] = {}
        self._peak_processes = 0
        self._first_cpu: dict[tuple[int, float], Decimal] = {}
        self._last_cpu: dict[tuple[int, float], Decimal] = {}
        self._names: dict[tuple[int, float], str] = {}

    def observe(self, snapshot: TreeSnapshot) -> None:
        """Fold one snapshot into the benchmark interval."""
        initial = self._observations == 0
        self._observations += 1
        totals: dict[str, tuple[int, int]] = {"total": (0, 0)}
        for identity, usage in snapshot.processes.items():
            name = _metric_name(usage.name)
            rss, fds = totals.get(name, (0, 0))
            totals[name] = (rss + usage.rss_bytes, fds + usage.fds)
            total_rss, total_fds = totals["total"]
            totals["total"] = (
                total_rss + usage.rss_bytes,
                total_fds + usage.fds,
            )
            if identity not in self._first_cpu:
                # A process born during the interval has spent all of its CPU
                # lifetime on this benchmark. Processes present at baseline
                # contribute only their subsequent delta.
                self._first_cpu[identity] = usage.cpu_seconds if initial else Decimal(0)
            self._last_cpu[identity] = usage.cpu_seconds
            self._names[identity] = name

        if initial:
            self._baseline = dict(totals)
        for name, values in totals.items():
            previous = self._peak.get(name, (0, 0))
            self._peak[name] = (max(previous[0], values[0]), max(previous[1], values[1]))
        self._peak_processes = max(self._peak_processes, len(snapshot.processes))

    def start(self) -> None:
        if self._root is None or self._thread is not None:
            raise RuntimeError("process resource sampler needs one live root and one start")
        self.observe(process_tree_snapshot(self._root))
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self) -> None:
        while not self._stop.wait(self._interval_seconds):
            if self._root is not None:
                self.observe(process_tree_snapshot(self._root))

    def stop(self) -> dict[str, dict]:
        if self._root is None or self._thread is None:
            raise RuntimeError("process resource sampler was not started")
        self._stop.set()
        self._thread.join()
        self.observe(process_tree_snapshot(self._root))
        self._thread = None
        return self.metrics()

    def metrics(self) -> dict[str, dict]:
        cpu: dict[str, Decimal] = {}
        for identity, last in self._last_cpu.items():
            name = self._names[identity]
            delta = max(Decimal(0), last - self._first_cpu[identity])
            cpu[name] = cpu.get(name, Decimal(0)) + delta
            cpu["total"] = cpu.get("total", Decimal(0)) + delta

        metrics = {}
        for name in sorted(self._peak):
            if name != "total" and name not in TRACKED_PROCESS_NAMES:
                continue
            peak_rss, peak_fds = self._peak[name]
            baseline_rss, baseline_fds = self._baseline.get(name, (0, 0))
            prefix = f"resources.{name}"
            metrics[f"{prefix}.cpu_s"] = {
                "unit": "seconds",
                "samples": [float(cpu.get(name, Decimal(0)))],
            }
            metrics[f"{prefix}.peak_rss_bytes"] = {
                "unit": "bytes",
                "samples": [peak_rss],
            }
            metrics[f"{prefix}.peak_rss_delta_bytes"] = {
                "unit": "bytes",
                "samples": [max(0, peak_rss - baseline_rss)],
            }
            metrics[f"{prefix}.peak_fds"] = {
                "unit": "count",
                "samples": [peak_fds],
            }
            metrics[f"{prefix}.peak_fd_delta"] = {
                "unit": "count",
                "samples": [max(0, peak_fds - baseline_fds)],
            }
        metrics["resources.total.peak_processes"] = {
            "unit": "count",
            "samples": [self._peak_processes],
        }
        return metrics
