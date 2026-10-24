"""Fresh owned-job sampled resources, with honest incomplete CPU semantics."""

from dataclasses import replace
from threading import Event
from types import SimpleNamespace
from typing import cast

import psutil
import pytest
from helpers.process_resources import ProcessTreeSampler
from helpers.process_snapshot import OwnedProcess, process_tree_snapshot

from tests.test_process_resource_sampler import snapshot, usage


def row(pid=11, *, cpu="1", rss=100, started_at=1.0):
    return usage(pid, "criterion", cpu=cpu, rss=rss, pss=80, uss=60, fds=3, started_at=started_at)


def fresh(**kwargs):
    process = SimpleNamespace(pid=11, create_time=lambda: 1.0)
    owned = OwnedProcess.capture(cast(psutil.Process, process))
    return ProcessTreeSampler(
        None,
        roots=lambda: (owned,),
        max_roots=1,
        fresh_lifetime=True,
        **{"max_processes": 3, "max_identities": 4, "max_observations": 8, **kwargs},
    )


def test_fresh_owned_job_counts_initial_cpu_instead_of_subtracting_startup():
    sampler = fresh()
    sampler.observe(snapshot(row(cpu="1.25")))
    sampler.observe(snapshot(row(cpu="1.75")))
    assert sampler.metrics()["resources.total.cpu_s"]["samples"] == [1.75]
    ordinary = ProcessTreeSampler(None)
    ordinary.observe(snapshot(row(cpu="1.25")))
    ordinary.observe(snapshot(row(cpu="1.75")))
    assert ordinary.metrics()["resources.total.cpu_s"]["samples"] == [0.5]


@pytest.mark.parametrize("rows", [[], [row(12)], [row(rss=0)]])
def test_root_gone_before_usable_initial_snapshot_never_yields_zero(rows):
    with pytest.raises(RuntimeError, match=r"root|usable"):
        fresh().observe(snapshot(*rows))


def test_short_lived_observed_child_keeps_last_counter_but_is_never_complete():
    sampler = fresh()
    sampler.observe(snapshot(row(cpu="1")))
    sampler.observe(snapshot(row(cpu="2"), row(12, cpu=".25", started_at=2.0)))
    sampler.observe(snapshot(row(cpu="3")))
    assert sampler.metrics()["resources.total.cpu_s"]["samples"] == [3.25]
    receipt = sampler.receipt()
    assert receipt["cpu_complete"] is False
    assert receipt["unseen_descendant_cpu_unavailable"] is True
    assert receipt["last_interval_unobserved"] is True
    assert receipt["rss_continuous_peak"] is False
    assert receipt["sample_count"] == 3 and len(receipt["observed_identities"]) == 2
    assert 0 < receipt["first_observed_ns"] <= receipt["last_observed_ns"]


def test_descendant_that_lives_between_snapshots_is_explicitly_unknown_not_zero():
    sampler = fresh()
    sampler.observe(snapshot(row(cpu="1")))
    sampler.observe(snapshot(row(cpu="2")))
    receipt = sampler.receipt()
    assert len(receipt["observed_identities"]) == 1
    assert receipt["unseen_descendant_cpu_unavailable"] and not receipt["cpu_complete"]


@pytest.mark.parametrize("limit", ["max_processes", "max_identities", "max_observations"])
def test_snapshot_churn_and_observation_budgets_fail_without_clipping(limit):
    sampler = fresh(**{limit: 1})
    sampler.observe(snapshot(row()))
    with pytest.raises(ValueError, match="budget"):
        sampler.observe(snapshot(row(cpu="2"), row(12, cpu=".5", started_at=2.0)))


def test_cumulative_churn_is_bounded_even_when_snapshots_are_small():
    sampler = fresh(max_processes=2, max_identities=2)
    sampler.observe(snapshot(row(), row(12, started_at=2.0)))
    sampler.observe(snapshot(row(cpu="2")))
    with pytest.raises(ValueError, match=r"identity.*budget"):
        sampler.observe(snapshot(row(cpu="3"), row(13, started_at=3.0)))


@pytest.mark.parametrize("cpu", [".5", "-1", "NaN", "Infinity"])
def test_bad_or_regressing_same_identity_cpu_never_clamps_to_zero(cpu):
    sampler = fresh()
    sampler.observe(snapshot(row(cpu="1")))
    with pytest.raises(ValueError, match="CPU"):
        sampler.observe(snapshot(row(cpu=cpu)))


def test_identity_key_and_root_birth_provenance_are_not_forged():
    sampler = fresh()
    with pytest.raises(ValueError, match="identity"):
        sampler.observe(type(snapshot())({(99, 1.0): row()}))
    with pytest.raises(ValueError, match=r"before.*root"):
        fresh().observe(snapshot(row(), row(12, started_at=0.5)))


@pytest.mark.parametrize("interval", [0, -1, float("inf"), float("nan"), True])
def test_sampling_interval_is_finite_positive(interval):
    with pytest.raises(ValueError):
        fresh(interval_seconds=interval)


@pytest.mark.parametrize("limit", [0, -1, True, None])
def test_fresh_mode_requires_finite_identity_and_sample_budgets(limit):
    with pytest.raises(ValueError):
        fresh(max_identities=limit)


def test_optional_memory_absence_is_not_reported_as_zero():
    sampler = fresh()
    sampler.observe(snapshot(replace(row(), pss_bytes=None, uss_bytes=None)))
    assert not any("pss" in key or "uss" in key for key in sampler.metrics())
    assert sampler.receipt()["optional_pss_available"] is False


def test_one_start_and_one_stop_only(monkeypatch):
    sampler = fresh()
    monkeypatch.setattr(sampler, "_snapshot", lambda: snapshot(row()))
    sampler.start()
    sampler.stop(timeout_seconds=1)
    with pytest.raises(RuntimeError):
        sampler.start()
    with pytest.raises(RuntimeError):
        sampler.stop(timeout_seconds=1)


def test_process_tree_budget_fails_before_recursive_expansion_or_counter_reads():
    child = SimpleNamespace(children=lambda **kwargs: pytest.fail("must not expand outside budget"))
    root = SimpleNamespace(children=lambda **kwargs: [child, child])
    with pytest.raises(ValueError, match="budget"):
        process_tree_snapshot(cast(psutil.Process, root), maximum=2)


def test_fresh_root_provider_cannot_change_to_an_existing_process():
    sampler = fresh()
    sampler._roots = lambda: ()
    with pytest.raises(ValueError, match="identity"):
        sampler._snapshot()


def test_bounded_live_snapshots_do_not_mix_zombie_cpu_clock_backends(monkeypatch):
    from tests.test_process_resource_snapshot import Process

    root = Process()
    monkeypatch.setattr(root, "children", lambda **kwargs: [])
    monkeypatch.setattr(root, "status", lambda: psutil.STATUS_ZOMBIE, raising=False)
    monkeypatch.setattr(
        root, "cpu_times", lambda: pytest.fail("terminal counters are not live samples")
    )
    assert process_tree_snapshot(cast(psutil.Process, root), maximum=1).processes == {}


def test_callback_read_failure_stays_failed_even_after_counter_recovers(monkeypatch):
    sampler = fresh(interval_seconds=10)
    monkeypatch.setattr(sampler, "_snapshot", lambda: snapshot(row()))
    sampler.start()

    def denied():
        raise psutil.AccessDenied(11)

    monkeypatch.setattr(sampler, "_snapshot", denied)
    with pytest.raises(psutil.AccessDenied):
        sampler.sample()
    monkeypatch.setattr(sampler, "_snapshot", lambda: snapshot(row()))
    with pytest.raises(RuntimeError, match="observation failed"):
        sampler.stop(timeout_seconds=1)


def test_stop_deadline_is_bounded_and_cannot_become_success_later(monkeypatch):
    sampler = fresh(interval_seconds=0.001)
    blocked, release = Event(), Event()
    calls = 0

    def read():
        nonlocal calls
        calls += 1
        if calls > 1:
            blocked.set()
            assert release.wait(2)
        return snapshot(row())

    monkeypatch.setattr(sampler, "_snapshot", read)
    sampler.start()
    assert blocked.wait(1)
    try:
        with pytest.raises(RuntimeError, match="stop within budget"):
            sampler.stop(timeout_seconds=0.01)
    finally:
        release.set()
        sampler._thread.join(1)
    with pytest.raises(RuntimeError, match="observation failed"):
        sampler.stop(timeout_seconds=1)
