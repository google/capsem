"""Pure network collector wiring: real fixtures and process discovery are forbidden."""

import json
import runpy
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parents[1]


def test_network_has_no_global_name_scan_or_custom_counter_backend():
    source = (ROOT / "benchmarks/collectors/network").read_text()
    assert "pgrep" not in source
    assert "ctypes" not in source
    assert "GUEST_HZ" not in source
    assert "host_cpu_seconds" not in source


def throughput_report(direction, streams, seconds):
    return {"throughput": {"direction": direction, "streams": streams, "seconds": seconds},
            "metrics": {
                "bytes_sent": {"unit": "bytes", "samples": [1000]},
                "bytes_received": {"unit": "bytes", "samples": [1000]},
                "elapsed_seconds": {"unit": "seconds", "samples": [1.23456789]},
                "streams": {"unit": "count", "samples": [streams]},
                "send_megabits_per_sec": {"unit": "megabits_per_second", "samples": [0.5]},
                "receive_megabits_per_sec": {"unit": "megabits_per_second", "samples": [0.5]},
                **({"round_trip_ms": {"unit": "milliseconds", "samples": [0.123456789, 0.25]}}
                   if direction == "latency" else {}),
            }}


def udp_report():
    return {"udp": {"count": 200, "size": 1400}, "metrics": {
        "sent": {"unit": "count", "samples": [200]},
        "received": {"unit": "count", "samples": [2]},
        "lost": {"unit": "count", "samples": [198]},
        "elapsed_seconds": {"unit": "seconds", "samples": [0.75]},
        "round_trip_ms": {"unit": "milliseconds", "samples": [0.125, 0.25]},
    }}


def run_collector(monkeypatch, *, failure=None):
    loaded = runpy.run_path(str(ROOT / "benchmarks/collectors/network"))
    main = loaded["main"]
    calls = []
    clients = iter(("a", "b"))
    counter = iter(range(1000))
    if failure == "counter_reset":
        counter = iter((100, 100, 90, 120, *range(1000)))
    class Client:
        def post(self, path, body):
            calls.append(("post", path))
            return {"id": "net" if path == "/networks" else next(clients)}
        def put(self, path):
            calls.append(("put", path))
        def get(self, path):
            if path.endswith("/logs"):
                return {"events": [{"event_type": "network.close", "event": {
                    "decision": {"reason": "unlinked"},
                    "frames": {"dropped": dict.fromkeys(loaded["DROPS"], index)},
                }} for index in (1, 2)]}
            return {"members": [{"vm_id": name, "state": "ready", "address": "10.0.0.2"}
                                for name in ("a", "b")]}
        def delete(self, path):
            calls.append(("delete", path))
    service = SimpleNamespace(start=lambda: calls.append(("start",)), client=Client,
                              stop=lambda: calls.append(("stop",)))
    def guest(client, vm, command, timeout=60):
        calls.append(("guest", command))
        if command == "cat /proc/net/snmp":
            value = -1 if failure == "negative_counter" else next(counter)
            return f"Tcp: RetransSegs\nTcp: {value}\n"
        if " throughput --direction " in command:
            direction = command.split("--direction ")[1].split()[0]
            streams = int(command.split("--streams ")[1].split()[0])
            seconds = int(command.split("--seconds ")[1].split()[0])
            if failure == "workload":
                raise RuntimeError("original workload failed")
            return json.dumps(throughput_report(direction, streams, seconds))
        if " udp --address " in command:
            return json.dumps(udp_report())
        return ""
    class Resources:
        def __init__(self, actual_service, settings):
            assert actual_service is service
        def start(self):
            calls.append(("sample_start",))
        def stop(self):
            calls.append(("sample_stop",))
            if failure == "resource":
                raise RuntimeError("resource observation failed")
            return {"resources.total.observed_cpu_lower_bound_s": {"unit": "seconds", "samples": [0.125]}}
        def receipt(self):
            return {"root": {"pid": 1234, "created_at": 1.0}, "sampler": {"cpu_complete": False}}
    monkeypatch.setitem(main.__globals__, "ServiceInstance", lambda: service)
    monkeypatch.setitem(main.__globals__, "guest", guest)
    monkeypatch.setitem(main.__globals__, "GuestResources", Resources)
    monkeypatch.setitem(main.__globals__, "wait_exec_ready", lambda *_a, **_k: True)
    monkeypatch.setitem(main.__globals__, "host_hops", lambda *_: pytest.fail("global discovery called"))
    monkeypatch.setitem(main.__globals__, "pump_ticks", lambda *_: pytest.fail("unstamped guest pump called"))
    monkeypatch.setitem(main.__globals__, "time", SimpleNamespace(
        monotonic=loaded["time"].monotonic, sleep=lambda *_: None))
    monkeypatch.setattr(sys, "argv", ["network", "--trials", "2", "--seconds", "1"])
    return main, calls


def test_every_lane_trial_preserves_raw_samples_and_owned_resources(monkeypatch, capsys):
    main, calls = run_collector(monkeypatch)
    assert main() == 0
    output = json.loads(capsys.readouterr().out)
    metrics = output["metrics"]
    for lane in ("tcp.upload.s1", "tcp.upload.s4", "tcp.download.s4", "tcp.latency", "udp"):
        assert metrics[lane + ".resources.total.observed_cpu_lower_bound_s"]["samples"] == [0.125, 0.125]
    for lane in ("tcp.upload.s1", "tcp.upload.s4", "tcp.download.s4"):
        assert metrics[lane + ".elapsed_seconds"]["samples"] == [1.23456789, 1.23456789]
        assert metrics[lane + ".retransmits"]["samples"] == [4, 4]
    assert metrics["tcp.round_trip_ms"]["samples"] == [0.123456789, 0.25] * 2
    assert metrics["udp.lost"]["samples"] == [198, 198]
    assert metrics["switch.dropped.short"]["samples"] == [3]
    sidecar = json.loads(output["sidecar"])
    assert len(sidecar["resource_windows"]) == 10
    assert sidecar["resource_windows"][0]["producer"]["bytes_sent"] == 1000
    assert not any(key.endswith((".bytes_sent", ".bytes_received", ".streams", ".sent", ".received")) for key in metrics)
    assert sidecar["unavailable"]["guest_pump_cpu"]
    assert sidecar["unavailable"]["out_of_tree_apple_vz"]
    assert not any(key.startswith("cpu.") for key in metrics)
    assert calls[-1] == ("stop",)
    active = False
    for call in calls:
        if call[0] == "sample_start":
            assert not active
            active = True
        elif call[0] == "sample_stop":
            assert active
            active = False
        elif call[0] == "guest" and ("--direction " in call[1] or " udp --address " in call[1]):
            assert active
    assert not active


def test_completed_work_counts_are_quality_facts_not_lower_is_better_costs():
    from capsem_builder.gate.config import load

    from benchmarks.helpers.network_observations import NetworkObservations

    observed = NetworkObservations(load(ROOT).benchmark.run)
    facts = observed.tcp("tcp.upload.s4", throughput_report("upload", 4, 1), direction="upload", streams=4, seconds=1)
    assert facts["bytes_sent"] == 1000 and facts["bytes_received"] == 1000
    assert not any(key.endswith((".bytes_sent", ".bytes_received", ".streams")) for key in observed.metrics)
    assert not any(key.endswith((".send_megabits_per_sec", ".receive_megabits_per_sec")) for key in observed.metrics)
    assert observed.metrics["tcp.upload.s4.megabits_per_sec"]["samples"] == [0.5]


@pytest.mark.parametrize("value", [True, 1.5, 2**53 + 1])
def test_aggregate_count_samples_remain_exact_in_existing_f64_store(value):
    from capsem_builder.gate.config import load

    from benchmarks.helpers.network_observations import NetworkObservations

    with pytest.raises(ValueError):
        NetworkObservations(load(ROOT).benchmark.run).add("switch.drops", "count", value)


@pytest.mark.parametrize("failure", ["resource", "workload", "negative_counter", "counter_reset"])
def test_network_failure_is_not_a_measurement_and_service_settles(monkeypatch, capsys, failure):
    main, calls = run_collector(monkeypatch, failure=failure)
    with pytest.raises((RuntimeError, ValueError)):
        main()
    assert calls[-1] == ("stop",)
    assert not capsys.readouterr().out
    if failure != "negative_counter":
        assert ("sample_stop",) in calls


@pytest.mark.parametrize("mutation", ["missing_bytes", "negative_bytes", "nan", "empty", "streams", "unit", "lost", "received"])
def test_original_network_reports_cannot_hide_partial_or_invalid_counters(mutation):
    from capsem_builder.gate.config import load

    from benchmarks.helpers.network_observations import NetworkObservations

    observed = NetworkObservations(load(ROOT).benchmark.run)
    if mutation in {"lost", "received"}:
        report = udp_report()
        report["metrics"][mutation]["samples"] = [1]
        with pytest.raises(ValueError):
            observed.udp(report, count=200)
    else:
        report = throughput_report("upload", 4, 1)
        if mutation == "missing_bytes":
            del report["metrics"]["bytes_sent"]
        elif mutation == "negative_bytes":
            report["metrics"]["bytes_sent"]["samples"] = [-1]
        elif mutation == "nan":
            report["metrics"]["elapsed_seconds"]["samples"] = [float("nan")]
        elif mutation == "empty":
            report["metrics"]["bytes_received"]["samples"] = []
        elif mutation == "streams":
            report["metrics"]["streams"]["samples"] = [1]
        else:
            report["metrics"]["bytes_sent"]["unit"] = "count"
        with pytest.raises(ValueError):
            observed.tcp("tcp.upload.s4", report, direction="upload", streams=4, seconds=1)


def test_original_resource_and_sample_budgets_are_not_replaced_by_unbounded_lists():
    from capsem_builder.gate.config import load

    from benchmarks.helpers.network_observations import NetworkObservations

    limits = load(ROOT).benchmark.run.model_copy(update={"max_samples_per_metric": 1})
    observed = NetworkObservations(limits)
    observed.add("rate", "seconds", 0.1)
    with pytest.raises(ValueError, match="budget"):
        observed.add("rate", "seconds", 0.2)
    limits = limits.model_copy(update={"max_collector_bytes": 1})
    with pytest.raises(ValueError, match="budget"):
        NetworkObservations(limits).window("tcp.upload.s1", 0, {"root": {"pid": 123}})


@pytest.mark.parametrize("missing", ["window", "resource_sample"])
def test_missing_trial_or_partial_optional_resource_series_is_not_complete(missing):
    from capsem_builder.gate.config import load

    from benchmarks.helpers.network_observations import NetworkObservations

    observed = NetworkObservations(load(ROOT).benchmark.run)
    for trial in range(2):
        if missing != "window" or trial == 0:
            observed.window("tcp.upload.s1", trial, {})
        if missing != "resource_sample" or trial == 0:
            observed.add("tcp.upload.s1.resources.total.sampled_pss_high_water_bytes", "bytes", 12)
    with pytest.raises(ValueError):
        observed.complete(("tcp.upload.s1",), trials=2)


def test_network_guest_text_uses_existing_byte_budget_before_json_parsing():
    loaded = runpy.run_path(str(ROOT / "benchmarks/collectors/network"))
    guest = loaded["guest"]
    settings = loaded["SETTINGS"]
    client = SimpleNamespace(post=lambda *_a, **_k: {
        "exit_code": 0, "stdout": {"encoding": "utf8", "data": "é"}})
    guest.__globals__["SETTINGS"] = settings.model_copy(update={
        "run": settings.run.model_copy(update={"max_collector_bytes": 2})})
    assert guest(client, "vm", "command") == "é"
    guest.__globals__["SETTINGS"] = settings.model_copy(update={
        "run": settings.run.model_copy(update={"max_collector_bytes": 1})})
    with pytest.raises(ValueError, match="budget"):
        guest(client, "vm", "command")


@pytest.mark.parametrize("page", [
    {}, {"events": [{}, {}, {}]}, {"events": [{}], "next_cursor": "again"},
])
def test_missing_or_unbounded_network_history_cannot_certify_drop_counts(page):
    loaded = runpy.run_path(str(ROOT / "benchmarks/collectors/network"))
    history = loaded["history"]
    settings = loaded["SETTINGS"]
    history.__globals__["SETTINGS"] = settings.model_copy(update={
        "run": settings.run.model_copy(update={"max_samples_per_metric": 2})})
    pages = iter((page, page))
    def get(path):
        try:
            return next(pages)
        except StopIteration:
            raise RuntimeError("source double exceeded finite page allowance") from None
    with pytest.raises(ValueError):
        history(SimpleNamespace(get=get), "network")
