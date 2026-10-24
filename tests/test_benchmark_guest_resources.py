"""Source doubles: disk/storage host resource window never launches a fixture."""

from __future__ import annotations

import json
import runpy
import sys
from decimal import Decimal
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.gate import config

ROOT = Path(__file__).resolve().parents[1]
SETTINGS = config.load(ROOT).benchmark.criterion.resources


def module():
    from benchmarks.helpers import guest_resources

    return guest_resources


class Process:
    pid = 12345
    created = 10.0
    state = "running"

    def create_time(self):
        return self.created

    def status(self):
        return self.state


class Sampler:
    def __init__(self, root, **kwargs):
        assert root is None
        self.kwargs = kwargs
        self.active = False

    def start(self):
        self.kwargs["roots"]()
        self.active = True

    def stop(self, *, timeout_seconds):
        assert timeout_seconds == SETTINGS.stop_timeout_seconds
        self.active = False
        self.kwargs["roots"]()
        return {
            f"resources.total.{name}": {"unit": unit, "samples": [value]}
            for name, unit, value in (
                ("cpu_s", "seconds", 0.25),
                ("peak_rss_bytes", "bytes", 900),
                ("peak_fds", "count", 4),
                ("peak_processes", "count", 2),
            )
        }

    def receipt(self):
        return {"cpu_complete": False, "optional_pss_available": False,
                "optional_uss_available": False, "sample_count": 2}


def observer(monkeypatch):
    helper = module()
    process = Process()
    service = SimpleNamespace(proc=SimpleNamespace(pid=process.pid, poll=lambda: None))
    patch_process(monkeypatch, helper, process)
    monkeypatch.setattr(helper, "ProcessTreeSampler", Sampler)
    return helper.GuestResources(service, SETTINGS), process, service


def patch_process(monkeypatch, helper, process):
    original = helper.psutil
    monkeypatch.setattr(helper, "psutil", SimpleNamespace(
        Process=lambda pid: process, STATUS_ZOMBIE=original.STATUS_ZOMBIE,
        NoSuchProcess=original.NoSuchProcess, AccessDenied=original.AccessDenied,
        Error=original.Error))


def test_adapter_uses_exact_original_root_and_existing_config_budgets(monkeypatch):
    resources, process, _ = observer(monkeypatch)
    resources.start()
    settings = resources.sampler.kwargs
    assert settings["fresh_lifetime"] is False
    assert settings["interval_seconds"] == SETTINGS.interval_seconds
    assert settings["max_roots"] == 1
    for name in ("max_processes", "max_identities", "max_observations"):
        assert settings[name] == getattr(SETTINGS, name)
    (root,) = settings["roots"]()
    assert (root.pid, root.started_at) == (process.pid, 10.0)
    metrics = resources.stop()
    assert metrics["resources.total.observed_cpu_lower_bound_s"]["samples"] == [0.25]
    assert metrics["resources.total.sampled_rss_high_water_bytes"]["samples"] == [900]
    assert metrics["resources.total.sampled_fds_high_water"]["samples"] == [4]
    assert not any("pss" in key or "uss" in key for key in metrics)
    receipt = resources.receipt()
    assert receipt["root"] == {"pid": process.pid, "created_at": 10.0}
    assert receipt["sampler"]["cpu_complete"] is False
    assert receipt["sampler"]["optional_pss_available"] is False
    assert receipt["budgets"] == SETTINGS.model_dump()
    assert receipt["budget_config"] == "benchmark.criterion.resources"
    assert "existing host service" in receipt["scope"]
    assert receipt["fd_complete"] is False


def test_missing_service_fails_before_sampler_creation():
    with pytest.raises(RuntimeError, match="original live service"):
        module().GuestResources(SimpleNamespace(proc=None), SETTINGS)


@pytest.mark.parametrize("phase", ["before_capture", "during_capture", "refresh"])
def test_retired_original_popen_cannot_be_rebound_to_a_reused_pid(monkeypatch, phase):
    helper = module()
    process = Process()  # A readable replacement at the SAME pid is not authority.
    service = SimpleNamespace(proc=SimpleNamespace(pid=process.pid, poll=lambda: None))
    patch_process(monkeypatch, helper, process)
    monkeypatch.setattr(helper, "ProcessTreeSampler", Sampler)
    if phase == "before_capture":
        service.proc.poll = lambda: 0
    elif phase == "during_capture":
        def capture(pid):
            service.proc.poll = lambda: 0
            return process
        helper.psutil.Process = capture
    if phase != "refresh":
        with pytest.raises(RuntimeError, match=r"original.*service"):
            helper.GuestResources(service, SETTINGS)
    else:
        resources = helper.GuestResources(service, SETTINGS)
        resources.start()
        service.proc.poll = lambda: 0
        with pytest.raises(RuntimeError, match=r"original.*service"):
            resources.stop()
        assert not resources.sampler.active
        with pytest.raises(RuntimeError):
            resources.receipt()


@pytest.mark.parametrize("failure", ["missing", "denied", "changed", "replaced", "pid", "zombie"])
def test_missing_denied_or_changed_original_root_fails(monkeypatch, failure):
    resources, process, service = observer(monkeypatch)
    resources.start()
    if failure in {"missing", "denied"}:
        error = module().psutil.NoSuchProcess if failure == "missing" else module().psutil.AccessDenied
        monkeypatch.setattr(process, "create_time", lambda: (_ for _ in ()).throw(error(process.pid)))
    elif failure == "changed":
        process.created = 11.0
    elif failure == "replaced":
        service.proc = SimpleNamespace(pid=process.pid)
    elif failure == "pid":
        service.proc.pid = 12346
    else:
        process.state = module().psutil.STATUS_ZOMBIE
    with pytest.raises((RuntimeError, ValueError, module().psutil.Error)):
        resources.stop()
    assert not resources.sampler.active
    with pytest.raises(RuntimeError):
        resources.receipt()


@pytest.mark.parametrize("failure", ["budget", "observation", "deadline", "missing_metric"])
def test_sampler_errors_or_partial_required_metrics_never_certify(monkeypatch, failure):
    resources, _, _ = observer(monkeypatch)
    resources.start()
    def stop(**kwargs):
        resources.sampler.active = False
        if failure == "missing_metric":
            return {}
        raise RuntimeError(failure)
    monkeypatch.setattr(resources.sampler, "stop", stop)
    with pytest.raises(RuntimeError):
        resources.stop()
    with pytest.raises(RuntimeError):
        resources.receipt()


def test_real_sampler_preserves_existing_service_cpu_origin_and_optional_absence(monkeypatch):
    from helpers import process_resources
    from helpers.process_snapshot import ProcessUsage, TreeSnapshot

    helper = module()
    process = Process()
    service = SimpleNamespace(proc=SimpleNamespace(pid=process.pid, poll=lambda: None))
    patch_process(monkeypatch, helper, process)
    observations = iter((Decimal(10), Decimal("10.25")))
    def snapshot(roots, *, maximum, maximum_processes):
        assert maximum == 1 and maximum_processes == SETTINGS.max_processes
        assert roots[0].pid == process.pid
        usage = ProcessUsage(process.pid, 10.0, "capsem-service", next(observations),
                             900, None, None, 4)
        return TreeSnapshot({(process.pid, 10.0): usage})
    monkeypatch.setattr(process_resources, "owned_process_snapshot", snapshot)
    monkeypatch.setattr(process_resources.threading, "Thread", lambda **kwargs: SimpleNamespace(
        start=lambda: None, join=lambda **_: None, is_alive=lambda: False))
    resources = helper.GuestResources(service, SETTINGS)
    resources.start()
    metrics = resources.stop()
    assert metrics["resources.total.observed_cpu_lower_bound_s"]["samples"] == [0.25]
    receipt = resources.receipt()["sampler"]
    assert receipt["observed_identities"][0]["cpu_origin_s"] == 10.0
    assert receipt["fresh_lifetime"] is False and receipt["sample_count"] == 2
    assert receipt["cpu_complete"] is False and receipt["rss_continuous_peak"] is False
    assert receipt["unseen_descendant_cpu_unavailable"] is True
    assert not any("pss" in key or "uss" in key for key in metrics)


def test_io_resource_selection_does_not_change_guest_cpu_allocation():
    from benchmarks.helpers.guest_runtime import LOAD_MODES, guest_cpus

    measured_modes = module().MEASURED_IO_MODES
    assert measured_modes == {"disk", "storage"}
    for mode in measured_modes:
        assert mode not in LOAD_MODES
        assert guest_cpus(mode, host_cpus=64) == 2


@pytest.mark.parametrize("mode", ["disk", "storage"])
@pytest.mark.parametrize("failure", [None, "workload", "resource", "request", "start"])
def test_collector_window_is_after_ready_before_exec_and_settled_before_retrieval(
    monkeypatch, capsys, mode, failure
):
    monkeypatch.syspath_prepend(str(ROOT / "guest/artifacts"))
    import benchmarks.helpers.storage_raw

    calls = []
    def post(path, body=None, **kwargs):
        assert body is not None
        if path == "/vms/create":
            calls.append("create")
            assert body["cpus"] == 2
            return {"id": "vm"}
        if body["command"].startswith("cat "):
            calls.append("artifact")
            return {"exit_code": 0, "stdout": {"encoding": "utf8", "data": "{}"}}
        calls.append("exec")
        if failure == "request":
            raise RuntimeError("request failure")
        return {"exit_code": 1 if failure == "workload" else 0}
    class Resources:
        def __init__(self, service, settings):
            assert settings == SETTINGS
        def start(self):
            calls.append("sample_start")
            if failure == "start":
                raise RuntimeError("initial root observation failure")
        def stop(self):
            calls.append("sample_stop")
            if failure == "resource":
                raise RuntimeError("resource failure")
            return {"resources.total.observed_cpu_lower_bound_s": {"unit": "seconds", "samples": [0.25]}}
        def receipt(self):
            calls.append("receipt")
            return {"sampler": {"cpu_complete": False}}
    main = runpy.run_path(str(ROOT / "benchmarks/collectors/_guest"))["main"]
    monkeypatch.setitem(main.__globals__, "ServiceInstance", lambda: SimpleNamespace(
        client=lambda: SimpleNamespace(post=post, delete=lambda *_: calls.append("delete")),
        start=lambda: None, stop=lambda: calls.append("service_stop")))
    monkeypatch.setitem(main.__globals__, "wait_exec_ready", lambda *_a, **_k: calls.append("ready") or True)
    monkeypatch.setitem(main.__globals__, "GuestResources", Resources)
    monkeypatch.setattr(benchmarks.helpers.storage_raw, "extract", lambda *_: (
        {"original.latency_ms": {"unit": "milliseconds", "samples": [0.123456789]}}, {}))
    monkeypatch.setattr(sys, "argv", [mode])
    if failure:
        with pytest.raises(RuntimeError):
            main()
        assert "artifact" not in calls
    else:
        assert main() == 0
        output = json.loads(capsys.readouterr().out)
        assert output["metrics"]["original.latency_ms"]["samples"] == [0.123456789]
        assert json.loads(output["sidecar"])["host_resources"]["sampler"]["cpu_complete"] is False
        assert calls.index("sample_stop") < calls.index("artifact")
    assert calls.index("ready") < calls.index("sample_start")
    if failure == "start":
        assert "exec" not in calls and "sample_stop" not in calls
    else:
        assert calls.index("sample_start") < calls.index("exec") < calls.index("sample_stop")
    assert calls[-2:] == ["delete", "service_stop"]
