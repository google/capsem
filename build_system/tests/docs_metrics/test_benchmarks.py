import copy

import pytest
from capsem_builder.docs_metrics.benchmarks import normalize


def snapshot():
    return {
        "source": {
            "release_label": "0.6.6",
            "source_commit": "a" * 40,
            "architecture": "x86_64",
            "artifact_created_at_utc": "2026-10-06T17:24:39Z",
            "workflow_run_url": "https://github.com/example/project/actions/runs/12",
        },
        "records": {
            "capsem-bench/data_0.6.6_x86_64.json": {
                "version": "0.3.0",
                "arch": "x86_64",
                "disk": {
                    "seq_read": {"throughput_mbps": 512},
                    "seq_write": {"throughput_mbps": 128},
                    "rand_read_4k": {"iops": 9000},
                    "rand_write_4k": {"iops": 500},
                },
            }
        },
    }


def test_release_identity_is_not_the_benchmark_format_version():
    data = normalize(snapshot())
    assert data["release"] == "0.6.6"
    assert data["architecture"] == "x86_64"
    chart = next(c for c in data["sections"]["io"]["charts"] if c["id"] == "disk-sequential")
    assert chart["unit"] == "MiB/s"
    assert chart["series"][0]["values"] == [512, 128]


@pytest.mark.parametrize(
    "flag,value", [("classification", "exploratory"), ("quick", True), ("doctor_exit", 1)]
)
def test_diagnostic_or_exploratory_snapshot_cannot_be_published(flag, value):
    data = snapshot()
    data["source"][flag] = value
    with pytest.raises(ValueError, match="eligible"):
        normalize(data)


def test_missing_cpu_and_memory_are_not_zero_measurements():
    data = normalize(snapshot())
    assert data["sections"]["resources"]["charts"] == []
    assert data["sections"]["resources"]["available"] is False


def test_nonfinite_numeric_results_fail_loudly():
    data = copy.deepcopy(snapshot())
    data["records"]["capsem-bench/data_0.6.6_x86_64.json"]["disk"]["seq_read"][
        "throughput_mbps"
    ] = float("nan")
    with pytest.raises(ValueError, match="finite"):
        normalize(data)


@pytest.mark.parametrize(
    "flag,value",
    [("classification", "development-diagnostic"), ("quick", True), ("doctor_exit", 1)],
)
def test_record_level_fitness_cannot_be_hidden_by_artifact_provenance(flag, value):
    data = snapshot()
    next(iter(data["records"].values()))[flag] = value
    with pytest.raises(ValueError, match="eligible"):
        normalize(data)


@pytest.mark.parametrize(
    "transport,label",
    [("service_uds", "Service UDS"), ("authenticated_gateway_tcp", "Authenticated gateway TCP")],
)
def test_route_chart_uses_recorded_ingress(transport, label):
    data = snapshot()
    data["records"]["route-latency/data.json"] = {
        "reader": {"transport": transport, "summary": {
            "p50_ms": 1.0, "p95_ms": 2.0, "p99_ms": 3.0, "samples": 160,
        }},
    }
    chart = next(c for c in normalize(data)["sections"]["logging"]["charts"] if c["id"] == "stats-reads")
    assert chart["series"][0]["name"] == label
    assert chart["series"][0]["values"] == [1.0, 2.0, 3.0]


@pytest.mark.parametrize("transport", [None, "unknown_path"])
def test_route_chart_never_guesses_unspecified_ingress(transport):
    data = snapshot()
    data["records"]["route-latency/data.json"] = {
        "reader": {"transport": transport, "summary": {
            "p50_ms": 1.0, "p95_ms": 2.0, "p99_ms": 3.0,
        }},
    }
    assert not normalize(data)["sections"]["logging"]["charts"]
