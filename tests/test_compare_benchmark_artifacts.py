import json
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from scripts.compare_benchmark_artifacts import (
    compare_values,
    collect_rows,
    latest_artifact,
    read_path,
)


def write_json(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data))


def test_latest_artifact_falls_back_to_legacy_mac_lifecycle_name(tmp_path):
    legacy = tmp_path / "benchmarks" / "lifecycle" / "data_1.2.3.json"
    write_json(legacy, {})

    assert latest_artifact(tmp_path, "lifecycle", "arm64") == legacy


def test_latest_artifact_prefers_arch_scoped_name(tmp_path):
    legacy = tmp_path / "benchmarks" / "fork" / "data_1.2.3.json"
    scoped = tmp_path / "benchmarks" / "fork" / "data_1.2.4_arm64.json"
    write_json(legacy, {})
    write_json(scoped, {})

    assert latest_artifact(tmp_path, "fork", "arm64") == scoped


def test_read_path_returns_none_for_missing_or_non_numeric_value():
    data = {"a": {"b": 42, "c": "nope"}}

    assert read_path(data, ("a", "b")) == 42.0
    assert read_path(data, ("a", "missing")) is None
    assert read_path(data, ("a", "c")) is None


def test_compare_values_reports_higher_better_gap():
    ratio, status = compare_values(25, 100, "higher")

    assert ratio == 0.25
    assert status == "75.0% lower"


def test_compare_values_reports_lower_better_gap():
    ratio, status = compare_values(250, 100, "lower")

    assert ratio == 2.5
    assert status == "150.0% slower"


def test_compare_values_reports_size_gap():
    ratio, status = compare_values(250, 100, "lower", "size")

    assert ratio == 2.5
    assert status == "150.0% larger"


def test_collect_rows_compares_common_artifacts_and_reports_missing_lanes(tmp_path):
    write_json(
        tmp_path / "benchmarks" / "capsem-bench" / "data_1.2.3_x86_64.json",
        {
            "disk": {
                "seq_write": {"throughput_mbps": 50},
                "seq_read": {"throughput_mbps": 40},
                "rand_write_4k": {"iops": 30},
                "rand_read_4k": {"iops": 20},
            },
            "rootfs": {"seq_read": {"throughput_mbps": 10}, "rand_read_4k": {"iops": 5}},
            "startup": {"commands": {"python3": {"mean_ms": 2}}},
        },
    )
    write_json(
        tmp_path / "benchmarks" / "capsem-bench" / "data_1.2.3_arm64.json",
        {
            "disk": {
                "seq_write": {"throughput_mbps": 100},
                "seq_read": {"throughput_mbps": 80},
                "rand_write_4k": {"iops": 60},
                "rand_read_4k": {"iops": 40},
            },
            "rootfs": {"seq_read": {"throughput_mbps": 20}, "rand_read_4k": {"iops": 10}},
            "startup": {"commands": {"python3": {"mean_ms": 1}}},
        },
    )
    write_json(tmp_path / "benchmarks" / "host-native" / "data_1.2.3_x86_64.json", {})

    rows, missing = collect_rows(tmp_path, "x86_64", "arm64")

    assert rows[0]["metric"] == "Scratch seq write"
    assert rows[0]["ratio"] == "0.50x"
    assert rows[0]["status"] == "50.0% lower"
    assert "host-native: missing arm64 artifact" in missing
