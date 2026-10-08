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

Host class is the same axis again. Every file was recorded on the build box,
and a GitHub-hosted runner ratcheted against it failed `fork_ms.min` at 1.22x
for a commit its previous hosted attempt had passed. Unlabelled files are
`local`; a hosted lane reads only hosted evidence, seeds until it has some, and
is held to its own config-owned factor. The gate tells each pytest step which
host it is on (`[benchmark_regression] host_class_variable`), because nothing
ambient reliably reaches a functional test.
"""

from __future__ import annotations

import json
import os
import subprocess
import tomllib
from enum import StrEnum
from pathlib import Path
from typing import Any

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


class HostClass(StrEnum):
    """The gate's `pytestsuite.HostClass`, as the test process reads it."""

    LOCAL = "local"
    HOSTED = "hosted"


def base_profile(project_root: Path) -> str | None:
    """The lane whose numbers the unlabelled historical evidence describes."""
    config = tomllib.loads((project_root / "config" / "gate.toml").read_text(encoding="utf-8"))
    return config["suites"]["pytest"].get("base_profile")


def measuring_host(project_root: Path) -> HostClass:
    """The host this test process is measuring on, as the gate stated it.

    Absent is a direct pytest run on a developer's machine, which is local. A
    value the enum does not know is refused rather than read as either.
    """
    variable = str(_regression_config(project_root)["host_class_variable"])
    return HostClass(os.environ.get(variable) or HostClass.LOCAL)


def latest_checked_in_benchmark(
    project_root: Path,
    category: BenchmarkCategory,
    profile: str | None = None,
    host: HostClass = HostClass.LOCAL,
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
        # Unlabelled is local for the same reason: the build box recorded all
        # of it, and no hosted number has ever been checked in without the field.
        if (document.get("host_class") or HostClass.LOCAL) != host:
            continue
        candidates.append((float(document["timestamp"]), path, document))
    if not candidates:
        if host != HostClass.LOCAL or (profile is not None and profile != base):
            # A lane with no evidence of its own is seeded by this run rather
            # than measured against another lane's. Returning the base
            # profile's numbers, or the build box's, here is exactly the
            # comparison this exists to stop making.
            return None
        raise AssertionError(f"no checked-in {category.value} benchmark evidence")
    return max(candidates, key=lambda row: (row[0], row[1].name))[2]


def maximum_factor(project_root: Path) -> float:
    return _regression(project_root, "maximum_factor")


def vm_lifecycle_factor(project_root: Path, host: HostClass = HostClass.LOCAL) -> float:
    if host == HostClass.HOSTED:
        return _regression(project_root, "hosted_vm_lifecycle_factor")
    return _regression(project_root, "vm_lifecycle_factor")


def _regression(project_root: Path, key: str) -> float:
    return float(_regression_config(project_root)[key])


def _regression_config(project_root: Path) -> dict[str, Any]:
    config = tomllib.loads((project_root / "config" / "gate.toml").read_text(encoding="utf-8"))
    return config["benchmark_regression"]


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
