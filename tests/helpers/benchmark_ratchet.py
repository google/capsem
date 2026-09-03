"""Evidence-derived guards for checked-in product benchmarks.

Evidence is scoped to the profile that produced it. It was not, and the
compatibility lane was measured against the base lane's numbers: `co-work`
carries more packages and a heavier rootfs, so its exec is honestly slower, and
comparing the two says nothing about whether anything regressed. That lane had
never run to completion in the recorded history, so the first time it did, a
1.23x "regression" was a profile difference wearing a ratchet's clothes.

The 18 files predating this carry no profile at all. They are read as the base
profile's, because that is the lane that recorded them -- which keeps the guard
enforced where it has always meant something, rather than dropping it for
everyone to make one lane pass.
"""

from __future__ import annotations

import json
import os
import subprocess
from enum import StrEnum
from pathlib import Path
from typing import Any

import tomllib
from pydantic import BaseModel, ConfigDict, Field


class HeadroomGuard(BaseModel):
    """One measured value and the operating margin its ceiling must retain."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    label: str = Field(min_length=1)
    measured: float = Field(ge=0, allow_inf_nan=False)
    ceiling: float = Field(gt=0, allow_inf_nan=False)
    minimum_factor: float = Field(gt=1, allow_inf_nan=False)
    accounting_slack: float = Field(default=0.0, ge=0, allow_inf_nan=False)
    unit: str = Field(min_length=1)

    @property
    def required_ceiling(self) -> float:
        return self.measured * self.minimum_factor

    @property
    def effective_ceiling(self) -> float:
        return self.ceiling + self.accounting_slack

    def verify(self) -> None:
        if self.required_ceiling > self.effective_ceiling:
            raise AssertionError(
                f"{self.label} leaves less than "
                f"{(self.minimum_factor - 1) * 100:.0f}% headroom: "
                f"measured={self.measured:.3f}{self.unit}, "
                f"ceiling={self.ceiling:.3f}{self.unit}, "
                f"accounting_slack={self.accounting_slack:.3f}{self.unit}, "
                f"required_ceiling={self.required_ceiling:.3f}{self.unit}"
            )


def assert_has_headroom(
    *,
    label: str,
    measured: float,
    ceiling: float,
    minimum_factor: float,
    unit: str,
    accounting_slack: float = 0.0,
) -> None:
    """Reject a passing measurement that has consumed its operating margin."""
    HeadroomGuard(
        label=label,
        measured=measured,
        ceiling=ceiling,
        minimum_factor=minimum_factor,
        accounting_slack=accounting_slack,
        unit=unit,
    ).verify()


class BenchmarkMetric(StrEnum):
    # These duration probes run on a shared host. The least-contended timing
    # sample is the repeatable product capability; scheduler or disk contention
    # can only make another sample slower. A real regression still raises every
    # floor. Image size is deterministic and therefore retains its maximum.
    LIFECYCLE_PROVISION = "operations.provision_ms.min"
    LIFECYCLE_READY = "operations.exec_ready_ms.min"
    LIFECYCLE_EXEC = "operations.exec_ms.min"
    LIFECYCLE_DELETE = "operations.delete_ms.min"
    FORK_DURATION = "fork.fork_ms.min"
    FORK_IMAGE_SIZE = "fork.image_size_mb.max"
    FORK_BOOT_PROVISION = "fork.boot_provision_ms.min"
    FORK_BOOT_READY = "fork.boot_ready_ms.min"


class BenchmarkCategory(StrEnum):
    LIFECYCLE = "lifecycle"
    FORK = "fork"


def base_profile(project_root: Path) -> str:
    """The lane whose numbers the unlabelled historical evidence describes."""
    config = tomllib.loads((project_root / "config" / "gate.toml").read_text(encoding="utf-8"))
    return str(config["suites"]["pytest"]["base_profile"])


def measuring_profile(project_root: Path) -> str:
    """The profile this test process is exercising."""
    return os.environ.get("CAPSEM_TEST_PROFILE") or base_profile(project_root)


def latest_checked_in_benchmark(
    project_root: Path,
    category: BenchmarkCategory,
    profile: str | None = None,
) -> dict[str, Any] | None:
    candidates: list[tuple[float, Path, dict[str, Any]]] = []
    # Only when a lane is actually being selected for. Unfiltered callers -- the
    # retention contract builds a fixture repository with no `config/` at all --
    # have no reason to need the project's profile set to read a directory.
    base = base_profile(project_root) if profile is not None else None
    evidence_dir = Path("benchmarks") / "baselines" / category.value
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
        # Unlabelled is the base profile's: every file that predates this field
        # was recorded by that lane, and reading them as nobody's would drop a
        # guard that has been meaningful since the first one was committed.
        if profile is not None and (document.get("profile") or base) != profile:
            continue
        candidates.append((float(document["timestamp"]), path, document))
    if not candidates:
        if profile is not None and profile != base:
            # A lane with no evidence of its own is seeded by this run rather
            # than measured against another lane's. Returning the base
            # profile's numbers here is exactly the comparison this exists to
            # stop making.
            return None
        raise AssertionError(f"no checked-in {category.value} benchmark evidence")
    return max(candidates, key=lambda row: (row[0], row[1].name))[2]


def maximum_factor(project_root: Path) -> float:
    return _regression(project_root, "maximum_factor")


def _regression(project_root: Path, key: str) -> float:
    config = tomllib.loads((project_root / "config" / "gate.toml").read_text(encoding="utf-8"))
    return float(config["benchmark_regression"][key])


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
