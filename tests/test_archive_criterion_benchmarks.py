import json
import importlib.util
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
SCRIPT_PATH = PROJECT_ROOT / "scripts" / "archive_criterion_benchmarks.py"
SPEC = importlib.util.spec_from_file_location("archive_criterion_benchmarks", SCRIPT_PATH)
archive_criterion_benchmarks = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(archive_criterion_benchmarks)

criterion_measurements = archive_criterion_benchmarks.criterion_measurements
split_full_id = archive_criterion_benchmarks.split_full_id


def _write_criterion_case(
    root: Path,
    directory: str,
    full_id: str,
    slope: float | None,
):
    out = root / directory / "new"
    out.mkdir(parents=True)
    (out / "benchmark.json").write_text(json.dumps({
        "full_id": full_id,
        "directory_name": directory,
        "throughput": None,
    }))
    base = slope if slope is not None else 100.0
    estimate = {
        "point_estimate": base,
        "standard_error": 1.0,
        "confidence_interval": {
            "confidence_level": 0.95,
            "lower_bound": base - 1,
            "upper_bound": base + 1,
        },
    }
    (out / "estimates.json").write_text(json.dumps({
        "slope": estimate if slope is not None else None,
        "mean": estimate | {"point_estimate": base + 10},
        "median": estimate | {"point_estimate": base + 20},
    }))


def test_criterion_measurements_reads_matching_prefixes(tmp_path):
    _write_criterion_case(
        tmp_path,
        "security_engine_cel_compile/host_contains_google",
        "security_engine_cel_compile/host_contains_google",
        18000.0,
    )
    _write_criterion_case(
        tmp_path,
        "unrelated/bench",
        "unrelated/bench",
        1.0,
    )

    measurements = criterion_measurements(
        tmp_path,
        ("security_engine_cel_compile/",),
    )

    assert measurements == [{
        "group": "security_engine_cel_compile",
        "name": "host_contains_google",
        "full_id": "security_engine_cel_compile/host_contains_google",
        "estimate_kind": "slope",
        "estimate_ns": 18000.0,
        "estimate_ci_ns": {
            "confidence_level": 0.95,
            "lower_bound": 17999.0,
            "upper_bound": 18001.0,
        },
        "estimate_standard_error_ns": 1.0,
        "slope_ns": 18000.0,
        "slope_ci_ns": {
            "confidence_level": 0.95,
            "lower_bound": 17999.0,
            "upper_bound": 18001.0,
        },
        "slope_standard_error_ns": 1.0,
        "mean_ns": 18010.0,
        "mean_ci_ns": {
            "confidence_level": 0.95,
            "lower_bound": 17999.0,
            "upper_bound": 18001.0,
        },
        "median_ns": 18020.0,
        "median_ci_ns": {
            "confidence_level": 0.95,
            "lower_bound": 17999.0,
            "upper_bound": 18001.0,
        },
    }]


def test_split_full_id_uses_last_slash_for_criterion_flat_dirs():
    assert split_full_id("security_engine_native_lookup/canonical_http_policy") == (
        "security_engine_native_lookup",
        "canonical_http_policy",
    )


def test_criterion_measurements_falls_back_to_mean_when_slope_is_null(tmp_path):
    _write_criterion_case(
        tmp_path,
        "security_engine_detection_ir_lowering/lower_and_compile_100_http_rules",
        "security_engine_detection_ir_lowering/lower_and_compile_100_http_rules",
        None,
    )

    measurements = criterion_measurements(
        tmp_path,
        ("security_engine_detection_ir_lowering/",),
    )

    assert measurements[0]["estimate_kind"] == "mean"
    assert measurements[0]["estimate_ns"] == 110.0
    assert "slope_ns" not in measurements[0]
