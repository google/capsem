import json
from pathlib import Path

from helpers.benchmark_artifacts import benchmark_output_path, enrich_benchmark_artifact


def test_benchmark_output_path_defaults_to_project_benchmarks(monkeypatch, tmp_path):
    monkeypatch.delenv("CAPSEM_BENCHMARK_OUTPUT_DIR", raising=False)
    monkeypatch.delenv("CAPSEM_BENCHMARK_RUN_ID", raising=False)

    root = tmp_path
    path = benchmark_output_path(root, "lifecycle", "1.2.3", "x86_64")

    assert path == root / "benchmarks" / "lifecycle" / "data_1.2.3_x86_64.json"


def test_benchmark_output_path_supports_output_dir_and_run_id(monkeypatch, tmp_path):
    out = tmp_path / "out"
    monkeypatch.setenv("CAPSEM_BENCHMARK_OUTPUT_DIR", str(out))
    monkeypatch.setenv("CAPSEM_BENCHMARK_RUN_ID", "20260529T053000Z")

    path = benchmark_output_path(tmp_path, "capsem-bench", "1.2.3", "x86_64")

    assert path == out / "capsem-bench" / "data_1.2.3_x86_64_20260529T053000Z.json"


def test_enrich_benchmark_artifact_records_host_and_commit(monkeypatch, tmp_path):
    monkeypatch.setenv("CAPSEM_BENCHMARK_RUN_ID", "run-1")
    data = enrich_benchmark_artifact(
        {"metric": 42},
        project_root=tmp_path,
        project_version="1.2.3",
        arch="x86_64",
        command="just benchmark",
    )

    assert data["schema"] == "capsem.benchmark-artifact.v1"
    assert data["project_version"] == "1.2.3"
    assert data["arch"] == "x86_64"
    assert data["recorded_at_utc"].endswith("+00:00")
    assert data["run_id"] == "run-1"
    assert data["command"] == "just benchmark"
    assert data["host"]["platform"]
    assert data["host"]["release"]
    assert data["host"]["cpu_count"] >= 1
    assert data["host"]["cpu_count_logical"] >= 1
    assert data["host"]["python_version"]
    assert data["git"]["commit"]
    assert data["git"]["source_dirty"] in (True, False)
    assert isinstance(data["git"]["dirty_paths"], list)
    assert data["metric"] == 42


def test_enrich_benchmark_artifact_does_not_mutate_input(tmp_path):
    original = {"nested": {"value": 1}}
    enriched = enrich_benchmark_artifact(
        original,
        project_root=tmp_path,
        project_version="1.2.3",
        arch="x86_64",
    )

    enriched["nested"]["value"] = 2
    assert original == {"nested": {"value": 1}}


def test_enriched_benchmark_artifact_is_json_serializable(tmp_path):
    data = enrich_benchmark_artifact(
        {"values": [1, 2, 3]},
        project_root=tmp_path,
        project_version="1.2.3",
        arch="x86_64",
    )

    json.loads(json.dumps(data))
