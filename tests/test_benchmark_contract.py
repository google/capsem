import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent


def test_just_benchmark_runs_full_canonical_artifact_suite():
    justfile = (PROJECT_ROOT / "justfile").read_text()
    recipe = justfile.split("\n# Backward-compatible alias for the canonical benchmark suite.", 1)[0]
    recipe = recipe.rsplit("\n# Run the standard artifact-recording benchmark suite.", 1)[1]

    assert "uv run python scripts/archive_superseded_benchmark_artifacts.py --archive-current-arch" in recipe
    assert "cargo bench -p capsem-security-engine --bench security_engine_cel" in recipe
    assert "cargo bench -p capsem-security-engine --bench detection_ir" in recipe
    assert "cargo bench -p capsem-core --bench mitm_pipeline" in recipe
    assert "cargo bench -p capsem-core --bench provider_model_parser" in recipe
    assert "uv run python scripts/archive_criterion_benchmarks.py" in recipe
    assert "uv run python -m pytest tests/capsem-serial/" in recipe
    assert "-m benchmark" in recipe
    assert "uv run python scripts/archive_superseded_benchmark_artifacts.py" in recipe


def test_capsem_bench_all_includes_storage_split_diagnostics():
    entrypoint = (PROJECT_ROOT / "guest" / "artifacts" / "capsem_bench" / "__main__.py").read_text()

    assert 'if mode in ("storage", "all"):' in entrypoint
    assert 'output["storage"] = storage_bench()' in entrypoint


def test_serial_benchmark_marker_collects_required_artifact_lanes():
    output = subprocess.check_output(
        [
            "uv",
            "run",
            "python",
            "-m",
            "pytest",
            "--collect-only",
            "-q",
            "tests/capsem-serial/",
            "-m",
            "benchmark",
        ],
        cwd=PROJECT_ROOT,
        text=True,
    )

    required_tests = {
        "test_capsem_bench_baseline.py::test_capsem_bench_baseline",
        "test_host_native_benchmark.py::test_host_native_benchmark",
        "test_lifecycle_benchmark.py::test_lifecycle_benchmark",
        "test_lifecycle_benchmark.py::test_fork_benchmark",
        "test_security_engine_benchmark.py::test_process_enforcement_benchmark_records_vm_originated_path",
        "test_security_engine_benchmark.py::test_http_request_enforcement_benchmark_records_vm_originated_path",
        "test_security_engine_benchmark.py::test_dns_request_enforcement_benchmark_records_vm_originated_path",
        "test_security_engine_benchmark.py::test_mcp_request_enforcement_benchmark_records_vm_originated_path",
    }

    for test_name in required_tests:
        assert test_name in output
