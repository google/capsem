"""The guest's MiB/s must become actual Mbit/s in the canonical store."""

import json
import runpy
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

from benchmarks.helpers.guest_measurements import numeric_metrics

COLLECTOR = Path(__file__).resolve().parents[1] / "benchmarks/collectors/_guest"


@pytest.mark.parametrize("mode", ["rootfs", "snapshot"])
def test_current_guest_byte_rates_are_normalized_without_changing_sidecar(mode, monkeypatch, capsys):
    document = {mode: {"seq_read": {"throughput_mbps": 4.0, "duration_ms": 500.0}}}
    client = SimpleNamespace(
        post=lambda path, *_, **__: (
            {"id": "vm"} if path == "/vms/create" else {
                "exit_code": 0, "stdout": {"encoding": "utf8", "data": json.dumps(document)},
            }
        ),
        delete=lambda *_: None,
    )
    main = runpy.run_path(str(COLLECTOR))["main"]
    monkeypatch.setitem(main.__globals__, "ServiceInstance", lambda: SimpleNamespace(
        client=lambda: client, start=lambda: None, stop=lambda: None,
    ))
    monkeypatch.setitem(main.__globals__, "wait_exec_ready", lambda *_, **__: True)
    monkeypatch.setattr(sys, "argv", [mode])
    assert main() == 0
    output = json.loads(capsys.readouterr().out)
    assert output["metrics"] == {
        "seq_read.duration_ms": {"unit": "milliseconds", "samples": [500.0]},
        "seq_read.throughput_mbit_s": {"unit": "megabits_per_second", "samples": [33.554432]},
    }
    assert json.loads(output["sidecar"]) == document


def test_true_network_bit_rates_are_not_scaled():
    assert numeric_metrics("mitm-load", {"throughput_mbps": 4.0}) == {
        "throughput_mbps": {"unit": "megabits_per_second", "samples": [4.0]}
    }


@pytest.mark.parametrize("mode", ["disk", "storage"])
def test_io_aggregate_byte_rates_keep_their_distinct_units(mode):
    assert numeric_metrics(mode, {"seq_read.throughput_mbps": 4.0}) == {
        "seq_read.throughput_mbit_s": {"unit": "megabits_per_second", "samples": [33.554432]},
    }


@pytest.mark.parametrize("mode", ["disk", "rootfs", "storage", "snapshot"])
def test_iops_are_operation_rates_without_rescaling_original_samples(mode):
    assert numeric_metrics(mode, {"rand_read.iops": 1234.5, "rand_write.iops": 67.25}) == {
        "rand_read.iops": {"unit": "operations_per_second", "samples": [1234.5]},
        "rand_write.iops": {"unit": "operations_per_second", "samples": [67.25]},
    }
