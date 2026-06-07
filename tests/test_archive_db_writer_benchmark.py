import importlib.util
import json
from pathlib import Path


PROJECT_ROOT = Path(__file__).parent.parent
SCRIPT = PROJECT_ROOT / "scripts" / "archive_db_writer_benchmark.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("archive_db_writer_benchmark", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _write_json(path, payload):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload))


def test_collect_db_writer_benchmark_parses_criterion_output(tmp_path):
    module = _load_module()
    root = tmp_path / "criterion" / "db_writer_pressure" / "file_events_128" / "new"
    _write_json(
        root / "benchmark.json",
        {
            "function_id": "file_events_128",
            "throughput": {"Elements": 128},
        },
    )
    _write_json(
        root / "estimates.json",
        {
            "mean": {
                "point_estimate": 1_500_000.0,
                "confidence_interval": {
                    "confidence_level": 0.95,
                    "lower_bound": 1_400_000.0,
                    "upper_bound": 1_600_000.0,
                },
            },
            "median": {
                "point_estimate": 1_250_000.0,
                "confidence_interval": {
                    "confidence_level": 0.95,
                    "lower_bound": 1_200_000.0,
                    "upper_bound": 1_300_000.0,
                },
            },
        },
    )
    _write_json(
        root / "sample.json",
        {
            "iters": [1.0, 1.0, 1.0, 1.0],
            "times": [1_000_000.0, 2_000_000.0, 3_000_000.0, 4_000_000.0],
            "sampling_mode": "Linear",
        },
    )

    data = module.collect_db_writer_benchmark(tmp_path / "criterion" / "db_writer_pressure")

    assert data["benchmark"] == "db_writer_pressure"
    assert data["rows"] == [
        {
            "name": "file_events_128",
            "burst_size": 128,
            "mean_ms": 1.5,
            "median_ms": 1.25,
            "events_per_sec_mean": 85333.3,
            "events_per_sec_median": 102400.0,
            "sample_percentiles": {
                "p50_ms": 2.5,
                "p95_ms": 3.85,
                "p99_ms": 3.97,
            },
            "mean_confidence": {
                "confidence_level": 0.95,
                "lower_ms": 1.4,
                "upper_ms": 1.6,
            },
            "median_confidence": {
                "confidence_level": 0.95,
                "lower_ms": 1.2,
                "upper_ms": 1.3,
            },
        }
    ]


def test_collect_db_writer_benchmark_requires_prior_criterion_run(tmp_path):
    module = _load_module()

    try:
        module.collect_db_writer_benchmark(tmp_path / "missing")
    except FileNotFoundError as exc:
        assert "cargo bench -p capsem-logger" in str(exc)
    else:
        raise AssertionError("missing Criterion output should fail loudly")
