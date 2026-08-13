"""Evidence-derived guards for checked-in product benchmarks."""

from __future__ import annotations

import json
import subprocess
import tomllib
from enum import StrEnum
from pathlib import Path
from typing import Any


class BenchmarkMetric(StrEnum):
    LIFECYCLE_PROVISION = "operations.provision_ms.mean"
    LIFECYCLE_READY = "operations.exec_ready_ms.mean"
    LIFECYCLE_EXEC = "operations.exec_ms.mean"
    LIFECYCLE_DELETE = "operations.delete_ms.mean"
    FORK_DURATION = "fork.fork_ms.mean"
    FORK_IMAGE_SIZE = "fork.image_size_mb.max"
    FORK_BOOT_PROVISION = "fork.boot_provision_ms.mean"
    FORK_BOOT_READY = "fork.boot_ready_ms.mean"


class BenchmarkCategory(StrEnum):
    LIFECYCLE = "lifecycle"
    FORK = "fork"


def latest_checked_in_benchmark(project_root: Path, category: BenchmarkCategory) -> dict[str, Any]:
    candidates: list[tuple[float, Path, dict[str, Any]]] = []
    evidence_dir = Path("benchmarks") / category.value
    tracked = subprocess.run(
        ["git", "ls-files", "-z", "--", str(evidence_dir)],
        cwd=project_root,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    for encoded in tracked:
        if not encoded:
            continue
        path = project_root / encoded.decode()
        if path.suffix != ".json":
            continue
        document = json.loads(path.read_text(encoding="utf-8"))
        candidates.append((float(document["timestamp"]), path, document))
    if not candidates:
        raise AssertionError(f"no checked-in {category.value} benchmark evidence")
    return max(candidates, key=lambda row: (row[0], row[1].name))[2]


def maximum_factor(project_root: Path) -> float:
    config = tomllib.loads((project_root / "config" / "gate.toml").read_text(encoding="utf-8"))
    return float(config["benchmark_regression"]["maximum_factor"])


def metric_value(document: dict[str, Any], metric: BenchmarkMetric) -> float:
    value: Any = document
    for component in metric.value.split("."):
        value = value[component]
    return float(value)


def assert_within_evidence(
    *,
    metric: BenchmarkMetric,
    current: float,
    baseline: dict[str, Any],
    factor: float,
) -> None:
    prior = metric_value(baseline, metric)
    allowed = prior * factor
    assert current <= allowed, (
        f"{metric.value} regressed {current / prior:.2f}x: "
        f"current={current:.2f}, baseline={prior:.2f}, allowed={allowed:.2f}"
    )
