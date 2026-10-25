"""Scoped resource wiring source doubles, never a Criterion workload."""

from pathlib import Path
from types import SimpleNamespace
from typing import TypedDict

import psutil
import pytest
from capsem_builder.gate import config
from helpers.process_resources import ProcessTreeSampler

from benchmarks.helpers import criterion_resources as subject
from tests.test_process_resource_sampler import snapshot, usage

ROOT = Path(__file__).resolve().parents[3]


class State(TypedDict):
    cpu: str
    rss: int
    status: str
    error: bool


@pytest.fixture
def observed(monkeypatch):
    settings = config.load(ROOT).benchmark.criterion.resources
    state: State = {"cpu": ".25", "rss": 100, "status": "running", "error": False}
    process = SimpleNamespace(pid=11, create_time=lambda: 1.0, status=lambda: state["status"])
    monkeypatch.setattr(
        subject, "psutil", SimpleNamespace(**{**vars(psutil), "Process": lambda pid: process})
    )

    def factory(*args, **kwargs):
        sampler = ProcessTreeSampler(*args, **kwargs)

        def read():
            if state["error"]:
                raise psutil.AccessDenied(11)
            return snapshot(
                usage(11, "criterion", cpu=state["cpu"], rss=state["rss"], pss=80, uss=60, fds=3)
            )

        monkeypatch.setattr(sampler, "_snapshot", read)
        return sampler

    monkeypatch.setattr(subject, "ProcessTreeSampler", factory)
    observer = subject.CriterionResources(settings)
    child = SimpleNamespace(pid=11, returncode=None)
    return observer, child, state


def test_measured_owned_lifetime_has_honest_units_and_retirement_receipt(observed):
    observer, child, state = observed
    observer.started(child)
    state["cpu"] = ".75"
    observer.before_wait(child)
    child.returncode = 0
    observer.retired(child)
    observer.close()
    metrics, receipt = observer.result()
    assert metrics["observed_cpu_lower_bound_s"] == {"unit": "seconds", "samples": [0.75]}
    assert metrics["sampled_rss_high_water_bytes"] == {"unit": "bytes", "samples": [100]}
    assert receipt["root"] == {"pid": 11, "created_at": 1.0}
    assert receipt["settled"] is True and receipt["return_code"] == 0
    assert 0 < receipt["started_ns"] <= receipt["retired_ns"] <= receipt["closed_ns"]
    assert receipt["sampler"]["cpu_complete"] is False
    assert receipt["sampler"]["unseen_descendant_cpu_unavailable"] is True


@pytest.mark.parametrize(
    "failure", ["zombie", "missing", "counter", "nonzero", "unsettled", "duplicate"]
)
def test_failed_or_incomplete_runtime_never_returns_resource_success(observed, failure):
    observer, child, state = observed
    if failure == "zombie":
        state["status"] = psutil.STATUS_ZOMBIE
    if failure == "missing":
        state["rss"] = 0
    if failure in ("zombie", "missing"):
        with pytest.raises(RuntimeError):
            observer.started(child)
        observer.close()
    else:
        observer.started(child)
        if failure == "counter":
            state["error"] = True
            with pytest.raises(psutil.AccessDenied):
                observer.before_wait(child)
            with pytest.raises((psutil.AccessDenied, RuntimeError)):
                observer.close()
        elif failure == "duplicate":
            with pytest.raises(RuntimeError):
                observer.started(child)
            observer.close()
        else:
            if failure == "nonzero":
                child.returncode = 7
                observer.retired(child)
            observer.close()
    with pytest.raises(RuntimeError):
        observer.result()


@pytest.mark.parametrize("pid", [11, 12])
def test_wrong_original_handle_cannot_supply_retirement_ack(observed, pid):
    observer, child, _ = observed
    observer.started(child)
    with pytest.raises(ValueError, match="identity"):
        observer.retired(SimpleNamespace(pid=pid, returncode=0))
    observer.close()
    with pytest.raises(RuntimeError):
        observer.result()


@pytest.mark.parametrize(
    "field,value",
    [
        ("interval_seconds", float("inf")),
        ("interval_seconds", float("nan")),
        ("stop_timeout_seconds", 0),
        ("max_processes", True),
        ("max_identities", 0),
        ("max_observations", -1),
    ],
)
def test_resource_configuration_rejects_unbounded_or_coerced_limits(field, value):
    settings = config.load(ROOT).benchmark.criterion.resources
    with pytest.raises(ValueError):
        type(settings).model_validate({**settings.model_dump(), field: value})


def test_missing_required_resource_field_is_not_a_partial_success(observed):
    observer, child, _ = observed
    observer.started(child)
    child.returncode = 0
    observer.retired(child)
    observer.close()
    observer._metrics.pop("resources.total.cpu_s")
    with pytest.raises(RuntimeError, match="required"):
        observer.result()
