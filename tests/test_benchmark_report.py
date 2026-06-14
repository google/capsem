import importlib.util
import json
from pathlib import Path

import pytest


PROJECT_ROOT = Path(__file__).parent.parent
SCRIPT = PROJECT_ROOT / "scripts" / "benchmark_report.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("benchmark_report", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_benchmark_report_extracts_nested_load_series(tmp_path):
    module = _load_module()
    artifact = tmp_path / "capsem-benchmark.json"
    artifact.write_text(json.dumps({
        "mcp_load": {
            "concurrency_levels": [{
                "concurrency": 64,
                "duration_s": 10.0,
                "total_requests": 50000,
                "errors": 0,
                "rps": 5000.0,
                "p50_ms": 1.0,
                "p95_ms": 2.0,
                "p99_ms": 3.0,
                "p999_ms": 4.0,
                "rss_peak_mb": 42.0,
            }],
        },
    }))

    series = module.load_series([artifact])

    assert len(series) == 1
    assert series[0].name == "mcp_load"
    assert series[0].levels[0].concurrency == 64
    assert series[0].levels[0].total_requests == 50_000


def test_benchmark_report_extracts_root_load_series(tmp_path):
    module = _load_module()
    path = tmp_path / "dns-load" / "baseline.json"
    path.parent.mkdir()
    path.write_text(json.dumps({
        "concurrency_levels": [{
            "concurrency": 64,
            "duration_s": 5.0,
            "total_requests": 60000,
            "errors": 0,
            "rps": 12000.0,
            "p50_ms": 0.8,
            "p95_ms": 1.0,
            "p99_ms": 1.2,
            "p999_ms": 2.0,
        }],
    }))

    series = module.load_series([path])

    assert series[0].name == "dns_load"
    assert series[0].levels[0].p99_ms == 1.2


def test_benchmark_report_extracts_mitm_local_count_series(tmp_path):
    module = _load_module()
    artifact = tmp_path / "mitm-local.json"
    artifact.write_text(json.dumps({
        "mitm_local": {
            "scenarios": [{
                "name": "model_json_response",
                "total_requests": 50000,
                "concurrency": 64,
                "successful": 50000,
                "failed": 0,
                "requests_per_sec": 4321.8,
                "latency_ms": {
                    "min": 0.3,
                    "max": 49.3,
                    "mean": 14.7,
                    "p50": 13.9,
                    "p95": 25.0,
                    "p99": 30.7,
                },
            }],
        },
    }))

    series = module.load_count_series([artifact])

    assert series[0].name == "mitm_local"
    assert series[0].scenarios[0].name == "model_json_response"
    assert series[0].scenarios[0].latency_ms.p99 == 30.7


def test_benchmark_report_rejects_invalid_rows(tmp_path):
    module = _load_module()
    artifact = tmp_path / "bad.json"
    artifact.write_text(json.dumps({
        "mcp_load": {
            "concurrency_levels": [{
                "concurrency": 0,
                "duration_s": 10.0,
                "total_requests": 1,
                "errors": 0,
                "rps": 1.0,
                "p50_ms": 1.0,
                "p95_ms": 1.0,
                "p99_ms": 1.0,
                "p999_ms": 1.0,
            }],
        },
    }))

    with pytest.raises(SystemExit, match="greater than 0"):
        module.load_series([artifact])
