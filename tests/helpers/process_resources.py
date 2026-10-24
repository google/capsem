"""Process-tree resource accounting for load benchmarks."""

from __future__ import annotations

import math
import re
import threading
import time
from collections.abc import Callable
from decimal import Decimal
from typing import cast

import psutil

from .process_snapshot import (
    OwnedProcess,
    TreeSnapshot,
    owned_process_snapshot,
    process_tree_snapshot,
)

SAMPLE_INTERVAL_SECONDS = 0.05
TRACKED_PROCESS_NAMES = frozenset(
    {
        "capsem_network",
        "capsem_gateway",
        "capsem_ledger",
        "capsem_process",
        "capsem_proxy",
        "capsem_router",
        "capsem_service",
        "capsem_tray",
    }
)


def _metric_name(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", name.lower()).strip("_")


class ProcessTreeSampler:
    """Track CPU deltas and peak resident resources without spawning tools."""

    def __init__(
        self,
        root: psutil.Process | None,
        *,
        interval_seconds: float = SAMPLE_INTERVAL_SECONDS,
        roots: Callable[[], tuple[OwnedProcess, ...]] | None = None,
        max_roots: int | None = None,
        fresh_lifetime: bool = False,
        max_processes: int | None = None,
        max_identities: int | None = None,
        max_observations: int | None = None,
    ) -> None:
        if roots is not None and (root is not None or type(max_roots) is not int or max_roots <= 0):
            raise ValueError("owned roots require exclusive provider and finite budget")
        if roots is None and max_roots is not None:
            raise ValueError("root budget requires an owned provider")
        if (
            type(interval_seconds) not in (int, float)
            or not math.isfinite(interval_seconds)
            or interval_seconds <= 0
        ):
            raise ValueError("sampling interval must be finite positive")
        bounds = (max_processes, max_identities, max_observations)
        if any(value is not None and (type(value) is not int or value <= 0) for value in bounds):
            raise ValueError("sampling budgets must be finite positive integers")
        if fresh_lifetime and (roots is None or any(value is None for value in bounds)):
            raise ValueError("fresh lifetime needs stamped roots and all sampling budgets")
        self._root = root
        self._roots, self._max_roots = roots, max_roots
        self._fresh = fresh_lifetime
        self._fresh_roots: tuple[OwnedProcess, ...] = ()
        if fresh_lifetime:
            assert roots is not None and max_roots is not None
            self._fresh_roots = roots()
            if (
                not isinstance(self._fresh_roots, tuple)
                or not 0 < len(self._fresh_roots) <= max_roots
                or any(not isinstance(root, OwnedProcess) for root in self._fresh_roots)
            ):
                raise ValueError("fresh lifetime requires exact nonempty owned roots")
        self._max_processes, self._max_identities, self._max_observations = bounds
        self._state_lock = threading.RLock()
        self._started = False
        self._first_observed_ns = self._last_observed_ns = 0
        self._interval_seconds = interval_seconds
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._observations = 0
        self._baseline: dict[str, tuple[int, int, int, int]] = {}
        self._peak: dict[str, tuple[int, int, int, int]] = {}
        self._peak_rss_breakdown: dict[str, tuple[int, int, int]] = {}
        self._pss_available = True
        self._uss_available = True
        self._rss_breakdown_available = True
        self._peak_processes = 0
        self._first_cpu: dict[tuple[int, float], Decimal] = {}
        self._last_cpu: dict[tuple[int, float], Decimal] = {}
        self._names: dict[tuple[int, float], str] = {}
        self._error: Exception | None = None

    def observe(self, snapshot: TreeSnapshot) -> None:
        """Fold one snapshot into the benchmark interval."""
        with self._state_lock:
            self._observe(snapshot)

    def _observe(self, snapshot: TreeSnapshot) -> None:
        initial = self._observations == 0
        if self._max_processes is not None and len(snapshot.processes) > self._max_processes:
            raise ValueError("process snapshot budget exceeded")
        if self._max_observations is not None and self._observations >= self._max_observations:
            raise ValueError("process observation budget exceeded")
        if (
            self._max_identities is not None
            and len(self._last_cpu.keys() | snapshot.processes.keys()) > self._max_identities
        ):
            raise ValueError("process identity churn budget exceeded")
        for identity, usage in snapshot.processes.items():
            if identity != (usage.pid, usage.started_at):
                raise ValueError("process snapshot identity is forged")
            if self._fresh and any(
                usage.started_at < root.started_at for root in self._fresh_roots
            ):
                raise ValueError("observed process born before fresh root")
            if (
                not usage.cpu_seconds.is_finite()
                or usage.cpu_seconds < 0
                or (identity in self._last_cpu and usage.cpu_seconds < self._last_cpu[identity])
            ):
                raise ValueError("process CPU counter invalid or regressed")
        if (
            self._fresh
            and initial
            and any(
                (root.pid, root.started_at) not in snapshot.processes
                or snapshot.processes[(root.pid, root.started_at)].rss_bytes <= 0
                for root in self._fresh_roots
            )
        ):
            raise RuntimeError("fresh root gone before usable initial snapshot")
        observed_ns = time.monotonic_ns()
        if initial:
            self._first_observed_ns = observed_ns
        self._last_observed_ns = observed_ns
        self._observations += 1
        totals: dict[str, tuple[int, int, int, int]] = {"total": (0, 0, 0, 0)}
        rss_breakdowns: dict[str, tuple[int, int, int]] = {"total": (0, 0, 0)}
        for identity, usage in snapshot.processes.items():
            name = _metric_name(usage.name)
            self._pss_available &= usage.pss_bytes is not None
            self._uss_available &= usage.uss_bytes is not None
            self._rss_breakdown_available &= all(
                value is not None
                for value in (
                    usage.rss_anon_bytes,
                    usage.rss_file_bytes,
                    usage.rss_shmem_bytes,
                )
            )
            pss = usage.pss_bytes or 0
            uss = usage.uss_bytes or 0
            rss_sum, pss_sum, uss_sum, fds_sum = totals.get(name, (0, 0, 0, 0))
            totals[name] = (
                rss_sum + usage.rss_bytes,
                pss_sum + pss,
                uss_sum + uss,
                fds_sum + usage.fds,
            )
            total_rss, total_pss, total_uss, total_fds = totals["total"]
            totals["total"] = (
                total_rss + usage.rss_bytes,
                total_pss + pss,
                total_uss + uss,
                total_fds + usage.fds,
            )
            rss_anon = usage.rss_anon_bytes or 0
            rss_file = usage.rss_file_bytes or 0
            rss_shmem = usage.rss_shmem_bytes or 0
            name_anon, name_file, name_shmem = rss_breakdowns.get(name, (0, 0, 0))
            rss_breakdowns[name] = (
                name_anon + rss_anon,
                name_file + rss_file,
                name_shmem + rss_shmem,
            )
            total_anon, total_file, total_shmem = rss_breakdowns["total"]
            rss_breakdowns["total"] = (
                total_anon + rss_anon,
                total_file + rss_file,
                total_shmem + rss_shmem,
            )
            if identity not in self._first_cpu:
                # A process born during the interval has spent all of its CPU
                # lifetime on this benchmark. Processes present at baseline
                # contribute only their subsequent delta.
                self._first_cpu[identity] = (
                    usage.cpu_seconds if initial and not self._fresh else Decimal(0)
                )
            self._last_cpu[identity] = usage.cpu_seconds
            self._names[identity] = name

        if initial:
            self._baseline = dict(totals)
        for name, values in totals.items():
            previous = self._peak.get(name, (0, 0, 0, 0))
            self._peak[name] = cast(
                tuple[int, int, int, int],
                tuple(max(old, new) for old, new in zip(previous, values, strict=True)),
            )
        for name, values in rss_breakdowns.items():
            previous = self._peak_rss_breakdown.get(name, (0, 0, 0))
            self._peak_rss_breakdown[name] = cast(
                tuple[int, int, int],
                tuple(max(old, new) for old, new in zip(previous, values, strict=True)),
            )
        self._peak_processes = max(self._peak_processes, len(snapshot.processes))

    def start(self) -> None:
        if (self._root is None and self._roots is None) or self._started:
            raise RuntimeError("process resource sampler needs one live root and one start")
        self._started = True
        self.sample()
        if not self._peak_processes:
            raise RuntimeError("process resource sampler found no live processes")
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    @property
    def active(self) -> bool:
        return self._thread is not None

    def _run(self) -> None:
        while not self._stop.wait(self._interval_seconds):
            try:
                self.sample()
            except Exception as error:
                # Deliver provider/identity/counter errors to the caller;
                # never let a background exception look like zero usage.
                self._error = error
                return

    def sample(self) -> None:
        # Serialize read+fold, not just fold: slow earlier reads cannot arrive
        # after newer counters and appear to regress. Callback reads share this.
        with self._state_lock:
            if self._error is not None:
                raise RuntimeError("process resource observation failed") from self._error
            try:
                self.observe(self._snapshot())
            except Exception as error:
                self._error = error
                raise

    def _snapshot(self) -> TreeSnapshot:
        if self._roots is not None:
            assert self._max_roots is not None
            roots = self._roots()
            if self._fresh and roots != self._fresh_roots:
                raise ValueError("fresh process roots changed identity")
            if self._max_processes is not None:
                return owned_process_snapshot(
                    roots, maximum=self._max_roots, maximum_processes=self._max_processes
                )
            return owned_process_snapshot(roots, maximum=self._max_roots)
        assert self._root is not None
        return process_tree_snapshot(self._root)

    def stop(self, *, timeout_seconds: float | None = None) -> dict[str, dict]:
        if (self._root is None and self._roots is None) or self._thread is None:
            raise RuntimeError("process resource sampler was not started")
        if timeout_seconds is not None and (
            not math.isfinite(timeout_seconds) or timeout_seconds <= 0
        ):
            raise ValueError("sampler stop timeout must be finite positive")
        self._stop.set()
        self._thread.join(timeout=timeout_seconds)
        if self._thread.is_alive():
            self._error = RuntimeError("process resource sampler failed to stop within budget")
            raise self._error
        self._thread = None
        if self._error is not None:
            raise RuntimeError("process resource observation failed") from self._error
        self.sample()
        return self.metrics()

    def receipt(self) -> dict:
        """Observed bounds, never a claim to unseen child CPU or exact peaks."""
        return {
            "sample_count": self._observations,
            "interval_seconds": self._interval_seconds,
            "first_observed_ns": self._first_observed_ns,
            "last_observed_ns": self._last_observed_ns,
            "fresh_lifetime": self._fresh,
            "cpu_complete": False,
            "rss_continuous_peak": False,
            "snapshot_atomic": False,
            "cpu_observations": "live process counters only; terminal/last-interval counters are not certified",
            "unseen_descendant_cpu_unavailable": True,
            "last_interval_unobserved": True,
            "optional_pss_available": self._pss_available,
            "optional_uss_available": self._uss_available,
            "observed_identities": [
                {
                    "pid": pid,
                    "created_at": created,
                    "cpu_origin_s": float(self._first_cpu[(pid, created)]),
                    "last_observed_cpu_s": float(value),
                }
                for (pid, created), value in self._last_cpu.items()
            ],
        }

    def metrics(self) -> dict[str, dict]:
        if not self._peak_processes:
            raise RuntimeError("process resource sampler recorded no processes")
        cpu: dict[str, Decimal] = {}
        for identity, last in self._last_cpu.items():
            name = self._names[identity]
            delta = last - self._first_cpu[identity]
            cpu[name] = cpu.get(name, Decimal(0)) + delta
            cpu["total"] = cpu.get("total", Decimal(0)) + delta

        metrics = {}
        for name in sorted(self._peak):
            if name != "total" and name not in TRACKED_PROCESS_NAMES:
                continue
            peak_rss, peak_pss, peak_uss, peak_fds = self._peak[name]
            baseline_rss, baseline_pss, baseline_uss, baseline_fds = self._baseline.get(
                name, (0, 0, 0, 0)
            )
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
            if self._rss_breakdown_available:
                peak_anon, peak_file, peak_shmem = self._peak_rss_breakdown[name]
                for category, value in (
                    ("anon", peak_anon),
                    ("file", peak_file),
                    ("shmem", peak_shmem),
                ):
                    metrics[f"{prefix}.peak_rss_{category}_bytes"] = {
                        "unit": "bytes",
                        "samples": [value],
                    }
            if self._pss_available:
                metrics[f"{prefix}.peak_pss_bytes"] = {
                    "unit": "bytes",
                    "samples": [peak_pss],
                }
                metrics[f"{prefix}.peak_pss_delta_bytes"] = {
                    "unit": "bytes",
                    "samples": [max(0, peak_pss - baseline_pss)],
                }
            if self._uss_available:
                metrics[f"{prefix}.peak_uss_bytes"] = {
                    "unit": "bytes",
                    "samples": [peak_uss],
                }
                metrics[f"{prefix}.peak_uss_delta_bytes"] = {
                    "unit": "bytes",
                    "samples": [max(0, peak_uss - baseline_uss)],
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
