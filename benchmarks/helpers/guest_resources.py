"""Bounded observations of the existing host service during guest I/O execution.

The sampler/counters remain tests.helpers owners. Reuse the configured sampler
budgets, not Criterion executable identity or a fresh process lifetime claim.
"""

from __future__ import annotations

import sys

import psutil
from capsem_builder.gate.benchmarkresourceschema import CriterionResourceConfig
from helpers.process_resources import ProcessTreeSampler
from helpers.process_snapshot import OwnedProcess
from helpers.service import ServiceInstance

MEASURED_IO_MODES = frozenset({"disk", "storage"})


class GuestResources:
    def __init__(self, service: ServiceInstance, settings: CriterionResourceConfig) -> None:
        if service.proc is None:
            raise RuntimeError("guest resource observation requires the original live service")
        self.service, self.process = service, service.proc
        self.settings = settings
        self.root = OwnedProcess.capture(psutil.Process(self.process.pid))
        self._complete = False
        self.sampler = ProcessTreeSampler(
            None, roots=self._roots, max_roots=1, fresh_lifetime=False,
            interval_seconds=settings.interval_seconds,
            max_processes=settings.max_processes,
            max_identities=settings.max_identities,
            max_observations=settings.max_observations,
        )

    def _roots(self) -> tuple[OwnedProcess, ...]:
        current = psutil.Process(self.root.pid)
        if (
            self.service.proc is not self.process
            or self.process.pid != self.root.pid
            or current.create_time() != self.root.started_at
            or current.status() == psutil.STATUS_ZOMBIE
        ):
            raise RuntimeError("original guest resource service root changed or retired")
        return (self.root,)

    def start(self) -> None:
        self.sampler.start()

    def stop(self) -> dict:
        # Stop joins the same bounded sampler, propagates worker errors, then
        # takes its final live observation before the caller retrieves JSON.
        self._complete = False
        metrics = self.sampler.stop(timeout_seconds=self.settings.stop_timeout_seconds)
        self._roots()
        required = ("cpu_s", "peak_rss_bytes", "peak_fds", "peak_processes")
        if any("resources.total." + name not in metrics for name in required):
            raise RuntimeError("required guest host-resource observations are missing")
        # Move the same sample objects; do not retain duplicate arrays/quantiles.
        measured = {
            key.rsplit(".", 1)[0] + "." + self._label(key.rsplit(".", 1)[1]): value
            for key, value in metrics.items()
        }
        self._complete = True
        return measured

    @staticmethod
    def _label(name: str) -> str:
        if name == "cpu_s":
            return "observed_cpu_lower_bound_s"
        if name.startswith("peak_"):
            stem = name.removeprefix("peak_")
            if stem.endswith("_bytes"):
                return "sampled_" + stem.removesuffix("_bytes") + "_high_water_bytes"
            return "sampled_" + stem + "_high_water"
        raise ValueError("unknown sampler resource field")

    def receipt(self) -> dict:
        if not self._complete:
            raise RuntimeError("guest host-resource observation did not complete")
        return {
            "root": {"pid": self.root.pid, "created_at": self.root.started_at},
            "scope": "existing host service and observed descendants during original guest exec only; excludes boot/artifact retrieval/teardown; no out-of-tree Apple VZ attribution",
            "budget_config": "benchmark.criterion.resources",
            "budgets": self.settings.model_dump(),
            "platform": sys.platform,
            "counter_owner": "process_cpu_seconds; backend resolution is platform dependent",
            "fd_complete": False,
            "fd_observations": "live sampled descriptor counts only; unseen peaks unavailable",
            "sampler": self.sampler.receipt(),
        }
