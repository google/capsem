"""Denied optional memory detail must not erase basic native accounting."""

from contextlib import nullcontext
from decimal import Decimal
from threading import Event
from types import SimpleNamespace
from typing import cast

import psutil
import pytest
from helpers import process_resources
from helpers.process_resources import ProcessTreeSampler
from helpers.process_snapshot import TreeSnapshot, process_tree_snapshot


class Process:
    pid = 99_999_999

    def __init__(self, optional_error=psutil.AccessDenied):
        self.optional_error = optional_error

    def children(self, *, recursive):
        assert recursive
        return []

    def oneshot(self):
        return nullcontext()

    def memory_full_info(self):
        raise self.optional_error()

    def memory_info(self):
        return SimpleNamespace(rss=1234)

    def create_time(self):
        return 1.0

    def name(self):
        return "capsem-service"

    def cpu_times(self):
        return SimpleNamespace(user=0.25, system=0.125)

    def num_fds(self):
        return 4


@pytest.mark.parametrize("optional_error", [psutil.AccessDenied, NotImplementedError])
def test_unavailable_extended_memory_retains_basic_metrics(optional_error):
    process = cast(psutil.Process, Process(optional_error))
    snapshot = process_tree_snapshot(process)
    (usage,) = snapshot.processes.values()
    assert usage.cpu_seconds == Decimal("0.375")
    assert usage.rss_bytes == 1234 and usage.fds == 4
    assert usage.pss_bytes is None and usage.uss_bytes is None
    sampler = ProcessTreeSampler(None)
    sampler.observe(snapshot)
    metrics = sampler.metrics()
    assert metrics["resources.total.peak_processes"]["samples"] == [1]
    assert metrics["resources.total.peak_rss_bytes"]["samples"] == [1234]
    assert not any("pss" in key or "uss" in key for key in metrics)


def test_denied_basic_accounting_is_not_a_zero_observation(monkeypatch):
    process = Process()

    def denied():
        raise psutil.AccessDenied()

    monkeypatch.setattr(process, "cpu_times", denied)
    with pytest.raises(psutil.AccessDenied):
        process_tree_snapshot(cast(psutil.Process, process))


def test_empty_observations_are_not_reported_as_zero_resource_usage():
    sampler = ProcessTreeSampler(None)
    sampler.observe(TreeSnapshot({}))
    with pytest.raises(RuntimeError, match="process"):
        sampler.metrics()


def test_empty_initial_snapshot_refuses_to_start_measurement(monkeypatch):
    monkeypatch.setattr(process_resources, "process_tree_snapshot", lambda _: TreeSnapshot({}))
    sampler = ProcessTreeSampler(cast(psutil.Process, Process()))
    with pytest.raises(RuntimeError, match="process"):
        sampler.start()


def test_denied_descendant_enumeration_invalidates_tree(monkeypatch):
    def denied(*, recursive):
        raise psutil.AccessDenied()

    process = Process()
    monkeypatch.setattr(process, "children", denied)
    with pytest.raises(psutil.AccessDenied):
        process_tree_snapshot(cast(psutil.Process, process))


def test_background_counter_failure_is_reported_even_if_final_read_recovers(monkeypatch):
    process = cast(psutil.Process, Process())
    valid = process_tree_snapshot(process)
    failed = Event()
    reads = 0

    def observe(_):
        nonlocal reads
        reads += 1
        if reads == 2:
            failed.set()
            raise psutil.AccessDenied()
        return valid

    monkeypatch.setattr(process_resources, "process_tree_snapshot", observe)
    sampler = ProcessTreeSampler(process, interval_seconds=0.001)
    sampler.start()
    assert failed.wait(timeout=1)
    with pytest.raises(RuntimeError, match="observation failed") as error:
        sampler.stop()
    assert isinstance(error.value.__cause__, psutil.AccessDenied)
