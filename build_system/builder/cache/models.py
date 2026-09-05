"""Strict immutable models for the cache control plane."""

from __future__ import annotations

import re
from enum import StrEnum
from pathlib import Path, PurePosixPath
from typing import Annotated

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    StrictBool,
    StrictInt,
    StringConstraints,
    model_validator,
)

from .contract import CacheContract, CacheScope, PruneStrategy
from .controlmodels import CacheControlPolicy
from .runtimemodels import RuntimeInventory, RuntimePolicy

PositiveStrictInt = Annotated[StrictInt, Field(gt=0)]
STAGE_ID = re.compile(r"^[a-z][a-z0-9-]*$")


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


class StagePolicy(CacheContract):
    """One independently accounted leaf in the cache tree."""

    path: Path
    external: StrictBool = False
    entry_root: Path = Path(".")
    selector_globs: tuple[str, ...] = ()
    maximum_age_hours: PositiveStrictInt
    maximum_count: PositiveStrictInt | None = None
    managed_globs: tuple[str, ...] = ("*",)
    lease_template: str | None = None

    @model_validator(mode="after")
    def validate_stage(self) -> StagePolicy:
        if self.path.is_absolute():
            if not self.external:
                raise ValueError("absolute stage path requires external=true")
            if self.prune_strategy is not PruneStrategy.EPHEMERAL:
                raise ValueError("external stage requires the ephemeral prune strategy")
            if len(self.path.parts) < 4:
                raise ValueError("external stage path must name a concrete descendant")
        else:
            if self.external:
                raise ValueError("external stage path must be absolute")
            object.__setattr__(self, "path", _relative_descendant(self.path, field="stage path"))
        entry_root = PurePosixPath(self.entry_root.as_posix())
        if self.entry_root.is_absolute() or ".." in entry_root.parts:
            raise ValueError("stage entry_root must stay inside the stage path")
        object.__setattr__(self, "entry_root", Path(entry_root))
        if self.scope is not CacheScope.DISK:
            raise ValueError("filesystem cache stages require repository scope")
        if self.prune_strategy in {PruneStrategy.DOCKER, PruneStrategy.TART}:
            raise ValueError("filesystem cache stages require a filesystem prune strategy")
        if not self.managed_globs or any(not pattern for pattern in self.managed_globs):
            raise ValueError("managed_globs must contain non-empty patterns")
        if self.lease_template is not None and self.lease_template.count("{key}") != 1:
            raise ValueError("lease_template must contain exactly one {key} placeholder")
        for pattern in self.selector_globs:
            path = PurePosixPath(pattern)
            if not pattern or path.is_absolute() or ".." in path.parts:
                raise ValueError("selector_globs must stay inside the cache root")
        return self


class CachePolicy(BaseModel):
    """The complete validated cache configuration."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    version: Annotated[StrictInt, Field(ge=1)]
    root: Path
    authority_environment: Annotated[str, StringConstraints(pattern=r"^[A-Z][A-Z0-9_]+$")]
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
            if runtime.log_stage not in self.stages:
                raise ValueError(
                    f"cache runtime {runtime_id!r} references unknown stage {runtime.log_stage!r}"
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
            child_ids = set(self.control.docker.images)
            duplicate_ids = child_ids & (set(self.stages) | set(self.runtimes))
            if duplicate_ids:
                raise ValueError(
                    "cache IDs must be globally unique: " + ", ".join(sorted(duplicate_ids))
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
