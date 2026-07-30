"""The checked-in benchmark history stays bounded without losing its two uses."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tomllib


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
