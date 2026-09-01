"""Strict immutable models for the cache control plane."""

from __future__ import annotations

import re
from enum import StrEnum
from pathlib import Path, PurePosixPath
from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field, StrictBool, StrictInt, model_validator

from .controlmodels import CacheControlPolicy
from .runtimemodels import RuntimeInventory, RuntimePolicy

PositiveStrictInt = Annotated[StrictInt, Field(gt=0)]
STAGE_ID = re.compile(r"^[a-z][a-z0-9-]*$")


class PruneMethod(StrEnum):
    """Supported retention algorithms."""

    LRU = "lru"
    GENERATIONAL = "generational"
    EPHEMERAL = "ephemeral"
    EXTERNAL = "external"
    NONE = "none"


class FocusGroup(StrEnum):
    """Existing public focused-test owners."""

    ASSETS = "assets"
    BENCHMARK = "benchmark"
    BINARIES = "binaries"
    FUNCTIONAL = "functional"
    INSTALL = "install"
    RELEASE_SYSTEM = "release-system"


class AdmissionEventKind(StrEnum):
    """Events that establish or reset the consecutive-force rail."""

    FORCED_ATTEMPT = "forced-attempt"
    COMPLETE_SUCCESS = "complete-success"


def _relative_descendant(value: Path, *, field: str) -> Path:
    posix = PurePosixPath(value.as_posix())
    if value.is_absolute() or str(posix) in {"", "."} or ".." in posix.parts:
        raise ValueError(f"{field} must be a relative descendant")
    return Path(posix)


class CalibrationPolicy(BaseModel):
    """Evidence requirements for later cap adjustments."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    minimum_samples: PositiveStrictInt
    warning_percentile: Annotated[StrictInt, Field(ge=1, le=100)]
    soft_generation_headroom: PositiveStrictInt
    hard_generation_headroom: PositiveStrictInt


class TestRoute(BaseModel):
    """One explicitly low-impact repository path prefix and its owners."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    prefix: str
    groups: tuple[FocusGroup, ...]

    @model_validator(mode="after")
    def validate_route(self) -> TestRoute:
        path = PurePosixPath(self.prefix)
        if self.prefix.startswith("/") or ".." in path.parts or not self.prefix:
            raise ValueError("test route prefix must be repository-relative")
        if not self.groups:
            raise ValueError("test route must name at least one focus group")
        return self


class TestAdmissionPolicy(BaseModel):
    """How often explicitly low-impact source may request complete proof."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    minimum_commits: PositiveStrictInt
    state_path: Path
    routes: tuple[TestRoute, ...]

    @model_validator(mode="after")
    def validate_admission(self) -> TestAdmissionPolicy:
        object.__setattr__(
            self, "state_path", _relative_descendant(self.state_path, field="admission state path")
        )
        prefixes = [route.prefix for route in self.routes]
        if len(prefixes) != len(set(prefixes)):
            raise ValueError("test admission route prefixes must be unique")
        return self


class StagePolicy(BaseModel):
    """One independently accounted leaf in the cache tree."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    path: Path
    warning_bytes: PositiveStrictInt
    soft_bytes: PositiveStrictInt
    hard_bytes: PositiveStrictInt
    prune: PruneMethod
    maximum_age_hours: PositiveStrictInt
    maximum_count: PositiveStrictInt | None = None
    managed_globs: tuple[str, ...] = ("*",)
    lease_template: str | None = None
    external: StrictBool = False

    @model_validator(mode="after")
    def validate_stage(self) -> StagePolicy:
        object.__setattr__(self, "path", _relative_descendant(self.path, field="stage path"))
        if not self.warning_bytes <= self.soft_bytes <= self.hard_bytes:
            raise ValueError("stage limits must satisfy warning_bytes <= soft_bytes <= hard_bytes")
        if self.external != (self.prune is PruneMethod.EXTERNAL):
            raise ValueError("external stages must use the external prune method")
        if not self.managed_globs or any(not pattern for pattern in self.managed_globs):
            raise ValueError("managed_globs must contain non-empty patterns")
        if self.lease_template is not None and self.lease_template.count("{key}") != 1:
            raise ValueError("lease_template must contain exactly one {key} placeholder")
        return self


class CachePolicy(BaseModel):
    """The complete validated cache configuration."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    version: Annotated[StrictInt, Field(ge=1)]
    root: Path
    minimum_free_bytes: PositiveStrictInt
    calibration: CalibrationPolicy = CalibrationPolicy(
        minimum_samples=5,
        warning_percentile=95,
        soft_generation_headroom=1,
        hard_generation_headroom=1,
    )
    test_admission: TestAdmissionPolicy = TestAdmissionPolicy(
        minimum_commits=10,
        state_path=Path("state/test-admission.jsonl"),
        routes=(),
    )
    stages: dict[str, StagePolicy]
    runtimes: dict[str, RuntimePolicy] = Field(default_factory=dict)
    control: CacheControlPolicy | None = None

    @model_validator(mode="after")
    def validate_policy(self) -> CachePolicy:
        object.__setattr__(self, "root", _relative_descendant(self.root, field="cache root"))
        if self.root != Path("cache"):
            raise ValueError("cache root must be the repository cache directory")
        if not self.stages:
            raise ValueError("cache policy must declare at least one stage")
        for stage_id in self.stages:
            if not STAGE_ID.fullmatch(stage_id):
                raise ValueError(f"invalid cache stage id: {stage_id!r}")
        for runtime_id, runtime in self.runtimes.items():
            if not STAGE_ID.fullmatch(runtime_id):
                raise ValueError(f"invalid cache runtime id: {runtime_id!r}")
            for stage_id in (runtime.receipt_stage, runtime.log_stage):
                if stage_id not in self.stages:
                    raise ValueError(
                        f"cache runtime {runtime_id!r} references unknown stage {stage_id!r}"
                    )
        if self.control is not None:
            docker_id = self.control.docker.runtime_id
            if docker_id not in self.runtimes:
                raise ValueError(f"cache control references unknown runtime {docker_id!r}")
            failure_stage = self.control.failure_artifacts.stage
            if failure_stage not in self.stages:
                raise ValueError(
                    f"cache control references unknown failure stage {failure_stage!r}"
                )
        items = sorted(self.stages.items())
        for index, (left_id, left) in enumerate(items):
            for right_id, right in items[index + 1 :]:
                overlaps = left.path == right.path or left.path in right.path.parents
                if overlaps or right.path in left.path.parents:
                    raise ValueError(
                        f"cache stage paths overlap: {left_id}={left.path} and "
                        f"{right_id}={right.path}"
                    )
        return self


class CacheEntry(BaseModel):
    """One independently removable generation beneath a stage root."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    key: str
    relative_path: Path
    logical_bytes: Annotated[StrictInt, Field(ge=0)]
    allocated_bytes: Annotated[StrictInt, Field(ge=0)]
    created_ns: Annotated[StrictInt, Field(ge=0)]
    last_used_ns: Annotated[StrictInt, Field(ge=0)]
    managed: StrictBool = True
    protected: StrictBool = False


class StageInventory(BaseModel):
    """Byte-accounted contents of one configured leaf stage."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    stage_id: str
    path: Path
    external: StrictBool
    logical_bytes: Annotated[StrictInt, Field(ge=0)]
    allocated_bytes: Annotated[StrictInt, Field(ge=0)]
    protected_bytes: Annotated[StrictInt, Field(ge=0)]
    entries: tuple[CacheEntry, ...]

    @property
    def entry_count(self) -> int:
        return len(self.entries)


class CacheInventory(BaseModel):
    """A point-in-time inventory of the repository cache."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    root: Path
    generated_ns: Annotated[StrictInt, Field(ge=0)]
    filesystem_free_bytes: Annotated[StrictInt, Field(ge=0)]
    logical_bytes: Annotated[StrictInt, Field(ge=0)]
    allocated_bytes: Annotated[StrictInt, Field(ge=0)]
    stages: tuple[StageInventory, ...]
    unclassified: tuple[CacheEntry, ...] = ()
    runtimes: tuple[RuntimeInventory, ...] = ()


class PruneAction(BaseModel):
    """One exact path selected by a pure retention plan."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    stage_id: str
    key: str
    path: Path
    logical_bytes: Annotated[StrictInt, Field(ge=0)]
    reason: str


class PrunePlan(BaseModel):
    """A deterministic deletion proposal, inert until explicitly applied."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    generated_ns: Annotated[StrictInt, Field(ge=0)]
    reclaim_bytes: Annotated[StrictInt, Field(ge=0)]
    actions: tuple[PruneAction, ...]
    violations: tuple[str, ...]


class ApplyResult(BaseModel):
    """Paths actually removed by the mutation boundary."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    removed: tuple[Path, ...]
    missing: tuple[Path, ...]
    journal: Path


class AdmissionDecision(BaseModel):
    """A complete-test admission answer with operator-facing evidence."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    allowed: StrictBool
    forced: StrictBool
    high_impact: StrictBool
    baseline: str | None
    target: str
    commits_since_success: Annotated[StrictInt, Field(ge=0)]
    changed_paths: tuple[str, ...]
    groups: tuple[FocusGroup, ...]
    explanation: str


class AdmissionEvent(BaseModel):
    """Durable force-attempt or successful-reset evidence."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    kind: AdmissionEventKind
    timestamp_ns: Annotated[StrictInt, Field(ge=0)]
    source_identity: str
    reason: str
