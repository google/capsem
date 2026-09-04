"""Strict contracts for cache state held by external runtimes."""

from __future__ import annotations

from enum import StrEnum
from pathlib import Path
from typing import Annotated, Literal

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    StrictBool,
    StrictInt,
    StrictStr,
    field_validator,
    model_validator,
)

from .contract import CacheContract, CacheScope, PruneStrategy

NonNegativeInt = Annotated[StrictInt, Field(ge=0)]
PositiveInt = Annotated[StrictInt, Field(gt=0)]


class RuntimeKind(StrEnum):
    DOCKER = "docker"
    TART = "tart"


class ResourceKind(StrEnum):
    IMAGE = "image"
    CONTAINER = "container"
    VOLUME = "volume"
    BUILD_CACHE = "build-cache"
    VM = "vm"


class RuntimeOperation(StrEnum):
    REMOVE_IMAGE = "remove-image"
    REMOVE_CONTAINER = "remove-container"
    REMOVE_VOLUME = "remove-volume"
    PRUNE_BUILD_CACHE = "prune-build-cache"
    CLEAR_BUILD_CACHE = "clear-build-cache"
    DELETE_VM = "delete-vm"


class DockerRuntimePolicy(CacheContract):
    kind: Literal["docker"]
    required: StrictBool = True
    command: StrictStr
    timeout_seconds: PositiveInt
    mutation_timeout_seconds: PositiveInt
    inventory_retry_attempts: PositiveInt
    inventory_retry_delay_milliseconds: NonNegativeInt
    log_stage: StrictStr
    image_prefixes: tuple[StrictStr, ...]
    container_prefixes: tuple[StrictStr, ...]
    volume_prefixes: tuple[StrictStr, ...]
    build_cache_owned: StrictBool
    maximum_age_hours: PositiveInt
    keep_image_generations: PositiveInt

    @field_validator("image_prefixes", "container_prefixes", "volume_prefixes", mode="before")
    @classmethod
    def arrays_are_frozen(cls, value: object) -> object:
        return tuple(value) if isinstance(value, list) else value

    @model_validator(mode="after")
    def ownership_is_explicit(self) -> DockerRuntimePolicy:
        if self.scope is not CacheScope.DOCKER or self.prune_strategy is not PruneStrategy.DOCKER:
            raise ValueError("Docker cache requires docker scope and prune strategy")
        if not self.command or any(character.isspace() for character in self.command):
            raise ValueError("Docker command must be one executable token")
        if not self.image_prefixes or not self.container_prefixes or not self.volume_prefixes:
            raise ValueError("Docker ownership prefixes must be non-empty")
        prefixes = (*self.image_prefixes, *self.container_prefixes, *self.volume_prefixes)
        if any(not prefix or prefix.isspace() for prefix in prefixes):
            raise ValueError("Docker ownership prefixes must contain visible text")
        return self


class TartRuntimePolicy(CacheContract):
    kind: Literal["tart"]
    required: StrictBool = True
    command: StrictStr
    timeout_seconds: PositiveInt
    mutation_timeout_seconds: PositiveInt
    log_stage: StrictStr
    vm_prefixes: tuple[StrictStr, ...]
    base_images: tuple[StrictStr, ...]
    home: StrictStr

    @field_validator("vm_prefixes", "base_images", mode="before")
    @classmethod
    def arrays_are_frozen(cls, value: object) -> object:
        return tuple(value) if isinstance(value, list) else value

    @model_validator(mode="after")
    def ownership_is_explicit(self) -> TartRuntimePolicy:
        if self.scope is not CacheScope.TART or self.prune_strategy is not PruneStrategy.TART:
            raise ValueError("Tart cache requires tart scope and prune strategy")
        if not self.command or any(character.isspace() for character in self.command):
            raise ValueError("Tart command must be one executable token")
        if not self.vm_prefixes or not self.base_images or not self.home:
            raise ValueError("Tart ownership, base images, and home must be non-empty")
        return self


RuntimePolicy = Annotated[
    DockerRuntimePolicy | TartRuntimePolicy,
    Field(discriminator="kind"),
]


class RuntimeCommandResult(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    argv: tuple[StrictStr, ...]
    returncode: StrictInt
    stdout: StrictStr
    stderr: StrictStr
    duration_ms: NonNegativeInt


class RuntimeCategory(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    name: StrictStr
    count: NonNegativeInt
    active: NonNegativeInt
    logical_bytes: NonNegativeInt
    reclaimable_bytes: NonNegativeInt


class RuntimeResource(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    kind: ResourceKind
    identity: StrictStr
    names: tuple[StrictStr, ...]
    logical_bytes: NonNegativeInt
    created_ns: NonNegativeInt
    last_used_ns: NonNegativeInt
    active: StrictBool
    owned: StrictBool
    protected: StrictBool


class RuntimeInventory(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    schema_id: Literal["capsem.runtime-inventory.v1"] = "capsem.runtime-inventory.v1"
    runtime_id: StrictStr
    kind: RuntimeKind
    available: StrictBool
    generated_ns: NonNegativeInt
    native_bytes: NonNegativeInt
    owned_bytes: NonNegativeInt
    error: StrictStr | None = None
    categories: tuple[RuntimeCategory, ...] = ()
    resources: tuple[RuntimeResource, ...] = ()


class RuntimeSnapshot(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    schema_id: Literal["capsem.runtime-snapshot.v1"] = "capsem.runtime-snapshot.v1"
    generated_ns: NonNegativeInt
    native_bytes: NonNegativeInt
    owned_bytes: NonNegativeInt
    runtimes: tuple[RuntimeInventory, ...]


class RuntimePruneAction(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    runtime_id: StrictStr
    operation: RuntimeOperation
    target: StrictStr
    logical_bytes: NonNegativeInt
    reason: StrictStr
    keep_bytes: NonNegativeInt | None = None
    maximum_age_hours: NonNegativeInt | None = None
    all_unused: StrictBool = False

    @model_validator(mode="after")
    def buildkit_budget_matches_operation(self) -> RuntimePruneAction:
        buildkit = self.operation is RuntimeOperation.PRUNE_BUILD_CACHE
        if buildkit != (self.keep_bytes is not None):
            raise ValueError("only bounded BuildKit prune actions declare keep_bytes")
        if not buildkit and (self.maximum_age_hours is not None or self.all_unused):
            raise ValueError("only bounded BuildKit prune actions declare prune scope")
        return self


class RuntimePrunePlan(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    generated_ns: NonNegativeInt
    reclaim_bytes: NonNegativeInt
    actions: tuple[RuntimePruneAction, ...]
    violations: tuple[StrictStr, ...]


class RuntimeActionResult(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    action: RuntimePruneAction
    returncode: StrictInt
    output: StrictStr


class RuntimeApplyResult(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    results: tuple[RuntimeActionResult, ...]
    journal: Path | None


class RuntimeMutationEvent(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    schema_id: Literal["capsem.runtime-cache-mutation.v1"]
    timestamp_ns: NonNegativeInt
    plan_generated_ns: NonNegativeInt
    reason: StrictStr
    results: tuple[RuntimeActionResult, ...]
