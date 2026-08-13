"""The checked-in benchmark history stays bounded without losing its two uses."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest
from helpers.benchmark_ratchet import (
    BenchmarkCategory,
    BenchmarkMetric,
    assert_within_evidence,
    latest_checked_in_benchmark,
    maximum_factor,
    metric_value,
)

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _pruner():
    script = PROJECT_ROOT / "scripts" / "prune-benchmark-history.py"
    spec = importlib.util.spec_from_file_location("prune_benchmark_history", script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


PRUNE = _pruner()


def _write(directory: Path, *names: str) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    for name in names:
        (directory / name).write_text("{}", encoding="utf-8")


def test_every_recording_of_the_current_release_is_retained(tmp_path: Path) -> None:
    """A floor derived from one sample is a guess, so the release being measured
    keeps its whole distribution."""
    _write(
        tmp_path / "capsem-bench",
        "data_1.6.1000001_arm64.json",
        "data_1.6.1000002_arm64.json",
        "data_1.6.1000003_arm64.json",
    )

    assert PRUNE.plan(tmp_path, (1, 6)) == []


def test_older_releases_keep_only_their_newest_recording(tmp_path: Path) -> None:
    _write(
        tmp_path / "capsem-bench",
        "data_1.3.1000001_arm64.json",
        "data_1.3.1000002_arm64.json",
        "data_1.5.1000003_arm64.json",
    )

    superseded = [path.name for path in PRUNE.plan(tmp_path, (1, 6))]

    assert superseded == ["data_1.3.1000001_arm64.json"]


def test_architectures_are_never_treated_as_samples_of_each_other(tmp_path: Path) -> None:
    """arm64 and x86_64 numbers are not comparable, so one must not supersede
    the other and leave a release with no recording for its own hardware."""
    _write(
        tmp_path / "capsem-bench",
        "data_1.3.1000001_arm64.json",
        "data_1.3.1000002_x86_64.json",
    )

    assert PRUNE.plan(tmp_path, (1, 6)) == []


def test_curated_baselines_are_never_pruned(tmp_path: Path) -> None:
    """baseline.json and friends are deliberate reference points, not routine
    output, and carry no timestamp to supersede them by."""
    _write(
        tmp_path / "mcp-load",
        "baseline.json",
        "baseline-pre-mitm-unification.json",
        "post_t3_debug_reference.json",
        "data_1.3.1000001_arm64.json",
        "data_1.3.1000002_arm64.json",
    )

    superseded = [path.name for path in PRUNE.plan(tmp_path, (1, 6))]

    assert superseded == ["data_1.3.1000001_arm64.json"]


def test_distinct_series_in_one_directory_do_not_supersede_each_other(
    tmp_path: Path,
) -> None:
    _write(
        tmp_path / "release-hermetic",
        "capsem_bench_all_1.0.1000001_arm64.json",
        "mcp_load_c1_16_64_1.0.1000002_arm64.json",
    )

    assert PRUNE.plan(tmp_path, (1, 6)) == []


def test_the_policy_tracks_the_workspace_version() -> None:
    """Retention follows Cargo.toml so the release being measured is always the
    one kept in full, without a second place to update."""
    # Asserted against Cargo.toml rather than a hardcoded floor. A literal here
    # is the second place the docstring warns about: `>= (1, 6)` outlived the
    # 1.6 line and failed the moment the workspace moved to 0.6.
    workspace = tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    declared = workspace["workspace"]["package"]["version"]
    expected = tuple(int(part) for part in declared.split(".")[:2])

    assert PRUNE.current_version(PROJECT_ROOT) == expected


def test_benchmark_regression_policy_is_relative_and_config_owned() -> None:
    factor = maximum_factor(PROJECT_ROOT)
    assert factor == 1.2

    baseline = {"fork": {"fork_ms": {"mean": 100.0}}}
    assert_within_evidence(
        metric=BenchmarkMetric.FORK_DURATION,
        current=120.0,
        baseline=baseline,
        factor=factor,
    )

    with pytest.raises(AssertionError, match=r"fork\.fork_ms\.mean regressed 1\.21x"):
        assert_within_evidence(
            metric=BenchmarkMetric.FORK_DURATION,
            current=121.0,
            baseline=baseline,
            factor=factor,
        )


def test_latest_benchmark_evidence_ignores_untracked_results(tmp_path: Path) -> None:
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    evidence = tmp_path / "benchmarks" / "fork"
    evidence.mkdir(parents=True)
    (evidence / "tracked.json").write_text(
        json.dumps({"timestamp": 1, "identity": "tracked"}), encoding="utf-8"
    )
    subprocess.run(["git", "add", "benchmarks/fork/tracked.json"], cwd=tmp_path, check=True)
    (evidence / "untracked.json").write_text(
        json.dumps({"timestamp": 2, "identity": "untracked"}), encoding="utf-8"
    )

    selected = latest_checked_in_benchmark(tmp_path, BenchmarkCategory.FORK)

    assert selected["identity"] == "tracked"


def test_release_benchmarks_use_typed_evidence_instead_of_authored_limits() -> None:
    source = (PROJECT_ROOT / "tests/capsem-serial/test_lifecycle_benchmark.py").read_text(
        encoding="utf-8"
    )
    for stale_limit in ("OP_GATE_MS", "FORK_GATE_MS", "IMAGE_SIZE_GATE_MB"):
        assert stale_limit not in source

    categories = {
        BenchmarkCategory.LIFECYCLE: (
            BenchmarkMetric.LIFECYCLE_PROVISION,
            BenchmarkMetric.LIFECYCLE_READY,
            BenchmarkMetric.LIFECYCLE_EXEC,
            BenchmarkMetric.LIFECYCLE_DELETE,
        ),
        BenchmarkCategory.FORK: (
            BenchmarkMetric.FORK_DURATION,
            BenchmarkMetric.FORK_IMAGE_SIZE,
            BenchmarkMetric.FORK_BOOT_PROVISION,
            BenchmarkMetric.FORK_BOOT_READY,
        ),
    }
    for category, metrics in categories.items():
        evidence = latest_checked_in_benchmark(PROJECT_ROOT, category)
        for metric in metrics:
            assert f"BenchmarkMetric.{metric.name}" in source
            assert metric_value(evidence, metric) > 0
