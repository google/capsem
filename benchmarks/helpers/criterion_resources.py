"""Adapt the existing sampler to one original measured executable lifetime.

Lifecycle authority remains processgroup.tee. These are observed CPU lower
bounds and sampled tree sums, never exact all-child totals or atomic peaks.
"""

from __future__ import annotations

import sys
import time

import psutil
from capsem_builder.gate.benchmarkresourceschema import CriterionResourceConfig
from helpers.process_resources import ProcessTreeSampler
from helpers.process_snapshot import OwnedProcess


class CriterionResources:
    def __init__(self, settings: CriterionResourceConfig) -> None:
        self.settings = settings
        self.sampler: ProcessTreeSampler | None = None
        self.root: OwnedProcess | None = None
        self.started_ns = self.retired_ns = self.closed_ns = 0
        self.return_code: int | None = None
        self.settled = False
        self._metrics: dict | None = None
        self._process = None

    def started(self, process) -> None:
        if self.started_ns or self.closed_ns:
            raise RuntimeError("runtime observer must start exactly once")
        self.started_ns = time.monotonic_ns()
        self._process = process
        owned = OwnedProcess.capture(psutil.Process(process.pid))
        self.root = owned
        if self.root.process.status() == psutil.STATUS_ZOMBIE:
            raise RuntimeError("root gone before usable runtime observation")
        self.sampler = ProcessTreeSampler(
            None,
            roots=lambda: (owned,),
            max_roots=1,
            fresh_lifetime=True,
            interval_seconds=self.settings.interval_seconds,
            max_processes=self.settings.max_processes,
            max_identities=self.settings.max_identities,
            max_observations=self.settings.max_observations,
        )
        self.sampler.start()

    def before_wait(self, process) -> None:
        if (
            self.root is None
            or process is not self._process
            or process.pid != self.root.pid
            or process.returncode is not None
        ):
            raise ValueError("runtime observation original identity/reap contract differs")
        assert self.sampler is not None
        self.sampler.sample()

    def retired(self, process) -> None:
        if (
            self.root is None
            or process is not self._process
            or process.pid != self.root.pid
            or process.returncode is None
            or self.settled
        ):
            raise ValueError("runtime retirement original identity differs")
        # Only the existing process owner calls this after strict descendant
        # settlement. This ACK is not a final CPU counter or peak-memory receipt.
        self.return_code = process.returncode
        self.retired_ns = time.monotonic_ns()
        self.settled = True

    def close(self) -> None:
        if self.closed_ns:
            raise RuntimeError("runtime observer already closed")
        try:
            if self.sampler is not None and self.sampler.active:
                metrics = self.sampler.stop(timeout_seconds=self.settings.stop_timeout_seconds)
                if self.settled and self.return_code == 0:
                    self._metrics = metrics
        finally:
            self.closed_ns = time.monotonic_ns()

    def result(self) -> tuple[dict, dict]:
        if (
            not self.closed_ns
            or self.root is None
            or self._metrics is None
            or not self.settled
            or self.return_code != 0
        ):
            raise RuntimeError("runtime resource observation did not complete successfully")
        labels = {
            "cpu_s": "observed_cpu_lower_bound_s",
            "peak_rss_bytes": "sampled_rss_high_water_bytes",
            "peak_pss_bytes": "sampled_pss_high_water_bytes",
            "peak_uss_bytes": "sampled_uss_high_water_bytes",
            "peak_fds": "sampled_fds_high_water",
            "peak_processes": "sampled_process_count_high_water",
        }
        required = ("cpu_s", "peak_rss_bytes", "peak_fds", "peak_processes")
        if any("resources.total." + name not in self._metrics for name in required):
            raise RuntimeError("required observed runtime resource fields are missing")
        metrics = {
            label: self._metrics["resources.total." + name]
            for name, label in labels.items()
            if "resources.total." + name in self._metrics
        }
        assert self.sampler is not None
        return metrics, {
            "root": {"pid": self.root.pid, "created_at": self.root.started_at},
            "started_ns": self.started_ns,
            "started_ns_semantics": "observer entered after original Popen returned; fresh child CPU origin is zero",
            "retired_ns": self.retired_ns,
            "closed_ns": self.closed_ns,
            "settled": self.settled,
            "return_code": self.return_code,
            "platform": sys.platform,
            "counter_owner": "process_cpu_seconds; backend resolution is platform dependent",
            "scope": "measured executable lifetime including Criterion setup/warmup/analysis; excludes prepare/sign/list/collector parsing",
            "sampler": self.sampler.receipt(),
        }
