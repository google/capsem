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

    # Unfiltered, so absence is an error rather than a lane waiting to be
    # seeded -- the function raises instead of returning None here.
    assert selected is not None
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
        assert evidence is not None, f"no checked-in {category.value} evidence at all"
        for metric in metrics:
            assert f"BenchmarkMetric.{metric.name}" in source
            assert metric_value(evidence, metric) > 0


def _evidence_repo(tmp_path: Path, files: dict[str, dict]) -> Path:
    """A tracked evidence directory, plus the config the lane names come from."""
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    evidence = tmp_path / "benchmarks" / "fork"
    evidence.mkdir(parents=True)
    for name, document in files.items():
        (evidence / name).write_text(json.dumps(document), encoding="utf-8")
        subprocess.run(["git", "add", f"benchmarks/fork/{name}"], cwd=tmp_path, check=True)
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text(
        '[suites.pytest]\nbase_profile = "code"\n', encoding="utf-8"
    )
    return tmp_path


def test_evidence_without_a_profile_is_read_as_the_base_lane_s(tmp_path: Path) -> None:
    """Every file predating the field was recorded by that lane.

    Reading them as nobody's would drop a guard that has meant something since
    the first one was committed, which is a worse answer than the one this
    replaces.
    """
    root = _evidence_repo(tmp_path, {"old.json": {"timestamp": 1, "identity": "unlabelled"}})

    selected = latest_checked_in_benchmark(root, BenchmarkCategory.FORK, "code")

    assert selected is not None and selected["identity"] == "unlabelled"


def test_a_lane_is_never_measured_against_another_lane_s_numbers(tmp_path: Path) -> None:
    """The failure this exists to stop.

    `co-work` carries more packages and a heavier rootfs, so its exec is
    honestly slower than `code`'s. Ratcheted against `code`'s evidence it
    reported a 1.23x regression that was a profile difference, and it did so
    the first time that lane ever ran to completion.
    """
    root = _evidence_repo(
        tmp_path,
        {
            "code.json": {"timestamp": 1, "identity": "code", "profile": "code"},
            "newer-code.json": {"timestamp": 9, "identity": "newer", "profile": "code"},
        },
    )

    assert latest_checked_in_benchmark(root, BenchmarkCategory.FORK, "co-work") is None, (
        "a lane with no evidence of its own is seeded by the run, not compared "
        "against the newest file some other lane happened to leave"
    )


def test_a_lane_selects_its_own_evidence_over_a_newer_foreign_one(tmp_path: Path) -> None:
    """Newest wins only within a lane. Across lanes it is not a comparison."""
    root = _evidence_repo(
        tmp_path,
        {
            "mine.json": {"timestamp": 1, "identity": "mine", "profile": "co-work"},
            "theirs.json": {"timestamp": 9, "identity": "theirs", "profile": "code"},
        },
    )

    selected = latest_checked_in_benchmark(root, BenchmarkCategory.FORK, "co-work")

    assert selected is not None and selected["identity"] == "mine"


# ---------------------------------------------------------------------------
# Semver. The clock-derived scheme -- `1.5.1783712334` -- was retired, and the
# pruner's filename pattern was not: it requires a six-digit third component,
# which semver never has. Under `0.6.0` every recording fell through to
# "curated baseline, never pruned", so the retention policy above described
# behaviour the tree no longer had.
# ---------------------------------------------------------------------------


def test_a_semver_recording_is_recognised_at_all(tmp_path: Path) -> None:
    """The bug in one assertion: an unrecognised file is immortal."""
    _write(tmp_path / "routes", "data_0.5.0_x86_64.json", "data_0.5.1_x86_64.json")

    superseded = [path.name for path in PRUNE.plan(tmp_path, (0, 6))]

    assert superseded == ["data_0.5.0_x86_64.json"], (
        "a semver recording was not recognised as routine output, so it can "
        "never be pruned; the history grows without bound and silently"
    )


def test_semver_patches_of_one_release_are_samples_of_it(tmp_path: Path) -> None:
    """`0.6.0` and `0.6.1` are the release being worked on, both kept.

    Retention groups by major and minor because that is what "this release"
    means here -- the same grouping the clock-derived scheme had.
    """
    _write(tmp_path / "routes", "data_0.6.0_x86_64.json", "data_0.6.1_x86_64.json")

    assert PRUNE.plan(tmp_path, (0, 6)) == []


def test_semver_recordings_are_ordered_by_patch_not_by_string(tmp_path: Path) -> None:
    """`0.5.10` is newer than `0.5.9`, which sorting as text gets backwards."""
    _write(tmp_path / "routes", "data_0.5.9_x86_64.json", "data_0.5.10_x86_64.json")

    superseded = [path.name for path in PRUNE.plan(tmp_path, (0, 6))]

    assert superseded == ["data_0.5.9_x86_64.json"]


def test_a_curated_baseline_is_still_never_pruned(tmp_path: Path) -> None:
    """Widening the pattern must not swallow the deliberate reference points.

    They are what "curated baseline" meant before the pattern started matching
    almost nothing, and the widening is the moment that could lose them.
    """
    _write(tmp_path / "routes", "baseline.json", "post_t3_debug_reference.json")

    assert PRUNE.plan(tmp_path, (0, 6)) == []


def test_a_dry_run_reports_what_would_remain() -> None:
    """The number a human reads before deciding to apply.

    It reported the count *before* pruning on a dry run and after it on a real
    one, under one label -- so "47 superseded ... -> 82 files" on a tree of 82
    read as "this changes nothing" while planning to delete more than half.
    """
    planned = PRUNE.summary(total=82, superseded=47, keep=(0, 6))
    assert "-> 35 files" in planned, planned


def test_the_summary_says_the_same_thing_either_way() -> None:
    """A dry run and the apply that follows it describe one outcome."""
    assert PRUNE.summary(total=82, superseded=47, keep=(0, 6)) == PRUNE.summary(
        total=82, superseded=47, keep=(0, 6)
    )
