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
    HostClass,
    assert_within_evidence,
    latest_checked_in_benchmark,
    maximum_factor,
    measuring_host,
    metric_value,
    vm_lifecycle_factor,
)

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _pruner():
    script = (
        PROJECT_ROOT
        / "build_system/builder/image/tools/build/prune_benchmark_history.py"
    )
    spec = importlib.util.spec_from_file_location("prune_benchmark_history", script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


PRUNE = _pruner()


def test_checked_in_baselines_have_a_dedicated_source_root() -> None:
    assert PRUNE.BENCHMARKS == PROJECT_ROOT / "benchmarks" / "baselines"


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


def test_benchmark_regression_policy_is_config_owned() -> None:
    factor = maximum_factor(PROJECT_ROOT)
    assert factor == 1.1

    baseline = {"fork": {"fork_ms": {"min": 100.0}}}
    assert_within_evidence(
        metric=BenchmarkMetric.FORK_DURATION,
        current=110.0,
        baseline=baseline,
        factor=factor,
    )

    with pytest.raises(AssertionError, match=r"fork\.fork_ms\.min regressed 1\.11x"):
        assert_within_evidence(
            metric=BenchmarkMetric.FORK_DURATION,
            current=111.0,
            baseline=baseline,
            factor=factor,
        )


def test_shared_host_timing_ratchets_use_the_least_contended_sample() -> None:
    """A shared-runner pause must not turn a capable operation into a regression."""
    current = {
        "operations": {
            name: {"min": 100.0, "mean": 200.0}
            for name in ("provision_ms", "exec_ready_ms", "exec_ms", "delete_ms")
        },
        "fork": {
            name: {"min": 100.0, "mean": 200.0}
            for name in ("fork_ms", "boot_provision_ms", "boot_ready_ms")
        }
    }
    baseline = {
        "operations": {
            name: {"min": 90.0, "mean": 100.0}
            for name in ("provision_ms", "exec_ready_ms", "exec_ms", "delete_ms")
        },
        "fork": {
            name: {"min": 90.0, "mean": 100.0}
            for name in ("fork_ms", "boot_provision_ms", "boot_ready_ms")
        }
    }

    for metric in (
        BenchmarkMetric.LIFECYCLE_PROVISION,
        BenchmarkMetric.LIFECYCLE_READY,
        BenchmarkMetric.LIFECYCLE_EXEC,
        BenchmarkMetric.LIFECYCLE_DELETE,
        BenchmarkMetric.FORK_DURATION,
        BenchmarkMetric.FORK_BOOT_PROVISION,
        BenchmarkMetric.FORK_BOOT_READY,
    ):
        assert_within_evidence(
            metric=metric,
            current=metric_value(current, metric),
            baseline=baseline,
            factor=1.2,
        )


def test_latest_benchmark_evidence_ignores_untracked_results(tmp_path: Path) -> None:
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    evidence = tmp_path / PRUNE.BENCHMARKS.relative_to(PROJECT_ROOT) / "fork"
    evidence.mkdir(parents=True)
    (evidence / "tracked.json").write_text(
        json.dumps({"timestamp": 1, "identity": "tracked"}), encoding="utf-8"
    )
    subprocess.run(
        ["git", "add", str((evidence / "tracked.json").relative_to(tmp_path))],
        cwd=tmp_path,
        check=True,
    )
    (evidence / "untracked.json").write_text(
        json.dumps({"timestamp": 2, "identity": "untracked"}), encoding="utf-8"
    )

    selected = latest_checked_in_benchmark(tmp_path, BenchmarkCategory.FORK)

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
        assert evidence is not None
        for metric in metrics:
            assert f"BenchmarkMetric.{metric.name}" in source
            assert metric_value(evidence, metric) > 0


# ---------------------------------------------------------------------------
# Host class. The fork and lifecycle evidence was recorded on the build box; a
# GitHub-hosted runner is different hardware, and ratcheting it against the
# build box's floor failed a release on `fork_ms.min` 212 ms against 173.9 ms
# for a commit whose previous hosted attempt had passed the same check.
# ---------------------------------------------------------------------------


def _evidence_repo(tmp_path: Path, files: dict[str, dict]) -> Path:
    """A tracked evidence directory, plus the config the lane names come from."""
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    evidence = tmp_path / PRUNE.BENCHMARKS.relative_to(PROJECT_ROOT) / "fork"
    evidence.mkdir(parents=True)
    for name, document in files.items():
        (evidence / name).write_text(json.dumps(document), encoding="utf-8")
        subprocess.run(
            ["git", "add", str((evidence / name).relative_to(tmp_path))],
            cwd=tmp_path,
            check=True,
        )
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text(
        '[suites.pytest]\nbase_profile = "code"\n', encoding="utf-8"
    )
    return tmp_path


def test_evidence_without_a_host_class_is_the_build_box_s(tmp_path: Path) -> None:
    """Every file predating the field was recorded locally, so reading it as
    local keeps the guard the build box has always enforced."""
    root = _evidence_repo(tmp_path, {"old.json": {"timestamp": 1, "identity": "unlabelled"}})

    local = latest_checked_in_benchmark(root, BenchmarkCategory.FORK, "code", HostClass.LOCAL)

    assert local is not None and local["identity"] == "unlabelled"
    assert latest_checked_in_benchmark(root, BenchmarkCategory.FORK, "code") == local, (
        "a caller that names no host class is the local lane, as it always was"
    )


def test_a_hosted_lane_is_never_measured_against_local_numbers(tmp_path: Path) -> None:
    root = _evidence_repo(
        tmp_path,
        {
            "unlabelled.json": {"timestamp": 1, "identity": "unlabelled", "profile": "code"},
            "local.json": {
                "timestamp": 9,
                "identity": "local",
                "profile": "code",
                "host_class": "local",
            },
        },
    )

    assert (
        latest_checked_in_benchmark(root, BenchmarkCategory.FORK, "code", HostClass.HOSTED)
        is None
    ), "a hosted lane with no hosted evidence seeds; it does not ratchet against the build box"


def test_a_hosted_lane_selects_its_own_evidence_over_newer_local_evidence(
    tmp_path: Path,
) -> None:
    root = _evidence_repo(
        tmp_path,
        {
            "hosted.json": {
                "timestamp": 1,
                "identity": "hosted",
                "profile": "code",
                "host_class": "hosted",
            },
            "local.json": {"timestamp": 9, "identity": "local", "profile": "code"},
            "hosted-co-work.json": {
                "timestamp": 5,
                "identity": "hosted-co-work",
                "profile": "co-work",
                "host_class": "hosted",
            },
        },
    )

    hosted = latest_checked_in_benchmark(root, BenchmarkCategory.FORK, "code", HostClass.HOSTED)
    local = latest_checked_in_benchmark(root, BenchmarkCategory.FORK, "code", HostClass.LOCAL)

    assert hosted is not None and hosted["identity"] == "hosted"
    assert local is not None and local["identity"] == "local", (
        "hosted evidence must never become the build box's floor either"
    )


def test_the_base_local_lane_still_refuses_to_have_no_evidence(tmp_path: Path) -> None:
    """Seeding is for a lane that has never recorded. The build box's base lane
    always has, so its evidence vanishing is a defect, not a fresh start."""
    root = _evidence_repo(
        tmp_path,
        {"hosted.json": {"timestamp": 1, "profile": "code", "host_class": "hosted"}},
    )

    with pytest.raises(AssertionError, match="no checked-in fork benchmark evidence"):
        latest_checked_in_benchmark(root, BenchmarkCategory.FORK, "code", HostClass.LOCAL)


def test_hosted_lanes_get_their_own_wider_factor_and_local_keeps_its_own() -> None:
    config = tomllib.loads((PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))
    regression = config["benchmark_regression"]

    assert vm_lifecycle_factor(PROJECT_ROOT) == 1.2
    assert vm_lifecycle_factor(PROJECT_ROOT, HostClass.LOCAL) == 1.2
    assert vm_lifecycle_factor(PROJECT_ROOT, HostClass.HOSTED) == (
        regression["hosted_vm_lifecycle_factor"]
    )
    assert regression["vm_lifecycle_factor"] < regression["hosted_vm_lifecycle_factor"] <= 2.0, (
        "the hosted envelope is wider than the build box's, and still a ratchet"
    )


def test_the_hosted_factor_absorbs_the_observed_hosted_spread() -> None:
    """Run 37432830392, one hosted runner, three samples each.

    The ratchet compares a run's least-contended sample with the evidence's,
    so what it has to absorb is how far one runner's floor sits from another's.
    One runner's own spread is the floor of that: 1.18x for fork and 1.19x for
    boot-ready, already at the build box's 1.2. The same commit then passed and
    failed `fork_ms.min` on consecutive attempts, at 1.22x of the evidence.
    """
    hosted = vm_lifecycle_factor(PROJECT_ROOT, HostClass.HOSTED)
    within_one_runner = {"fork_ms": (212.0, 251.0), "boot_ready_ms": (509.0, 604.0)}
    for slowest, fastest in ((high, low) for low, high in within_one_runner.values()):
        assert slowest / fastest * 1.2 <= hosted, (
            "a second runner's floor must be able to sit a whole run's spread "
            "beyond the first's, with the build box's margin left over"
        )

    baseline = {"fork": {"fork_ms": {"min": 173.9}}}
    with pytest.raises(AssertionError, match=r"regressed 1\.22x"):
        assert_within_evidence(
            metric=BenchmarkMetric.FORK_DURATION,
            current=212.0,
            baseline=baseline,
            factor=vm_lifecycle_factor(PROJECT_ROOT, HostClass.LOCAL),
        )
    assert_within_evidence(
        metric=BenchmarkMetric.FORK_DURATION,
        current=212.0,
        baseline=baseline,
        factor=hosted,
    )


def test_the_host_class_comes_from_the_gate_owned_variable(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config = tomllib.loads((PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))
    variable = config["benchmark_regression"]["host_class_variable"]

    monkeypatch.delenv(variable, raising=False)
    assert measuring_host(PROJECT_ROOT) is HostClass.LOCAL, (
        "a direct pytest run with no gate is the developer's own machine"
    )
    monkeypatch.setenv(variable, "hosted")
    assert measuring_host(PROJECT_ROOT) is HostClass.HOSTED
    monkeypatch.setenv(variable, "local")
    assert measuring_host(PROJECT_ROOT) is HostClass.LOCAL
    monkeypatch.setenv(variable, "laptop")
    with pytest.raises(ValueError):
        measuring_host(PROJECT_ROOT)


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


def test_evidence_tolerance_and_route_budgets_are_separate_knobs() -> None:
    """One number meant two things, and moving it moved both.

    `[benchmark_regression] maximum_factor` is how far a metric may drift from
    checked-in evidence. `test_route_health.py` read the same value as the
    headroom over an authored hot-route budget. They answer different
    questions -- "has this changed" and "is this fast enough" -- so widening
    tolerance while chasing a flaky comparison also quietly relaxed every
    route budget in the ironbank gate.
    """
    config = tomllib.loads(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )

    assert "minimum_headroom_factor" in config["benchmark"]["route_health"], (
        "the route-health ceilings have no minimum operating-margin policy"
    )
    source = (PROJECT_ROOT / "tests" / "helpers" / "route_health_budget.py").read_text(
        encoding="utf-8"
    )
    assert "benchmark.route_health" in source, (
        "the route-health budget library does not read its own typed config"
    )
    assert config["benchmark_regression"]["minimum_time_resolution_ms"] == 1.0
    assert set(config["benchmark_regression"]) == {
        "maximum_factor",
        "minimum_time_resolution_ms",
        "vm_lifecycle_factor",
        "hosted_vm_lifecycle_factor",
        "host_class_variable",
        "hosted_environment",
    }
    # Whole-boot timings get their own, wider envelope; it must never be the
    # one the rest of the product is held to.
    regression = config["benchmark_regression"]
    assert 1 < regression["maximum_factor"] <= regression["vm_lifecycle_factor"]
