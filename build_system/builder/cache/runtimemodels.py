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

NonNegativeInt = Annotated[StrictInt, Field(ge=0)]
PositiveInt = Annotated[StrictInt, Field(gt=0)]


class RuntimeKind(StrEnum):
    DOCKER = "docker"
    TART = "tart"


class ResourceKind(StrEnum):
    IMAGE = "image"
    CONTAINER = "container"
    BUILD_CACHE = "build-cache"
    VM = "vm"


class RuntimeOperation(StrEnum):
    REMOVE_IMAGE = "remove-image"
    REMOVE_CONTAINER = "remove-container"
    PRUNE_BUILD_CACHE = "prune-build-cache"
    DELETE_VM = "delete-vm"


class DockerRuntimePolicy(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    kind: Literal["docker"]
    command: StrictStr
    timeout_seconds: PositiveInt
    receipt_stage: StrictStr
    log_stage: StrictStr
    image_prefixes: tuple[StrictStr, ...]
    container_prefixes: tuple[StrictStr, ...]
    build_cache_owned: StrictBool
    maximum_age_hours: PositiveInt
    keep_image_generations: PositiveInt
    build_cache_keep_bytes: PositiveInt

    @field_validator("image_prefixes", "container_prefixes", mode="before")
    @classmethod
    def arrays_are_frozen(cls, value: object) -> object:
        return tuple(value) if isinstance(value, list) else value

    @model_validator(mode="after")
    def ownership_is_explicit(self) -> DockerRuntimePolicy:
        if not self.command or any(character.isspace() for character in self.command):
            raise ValueError("Docker command must be one executable token")
        if not self.image_prefixes or not self.container_prefixes:
            raise ValueError("Docker ownership prefixes must be non-empty")
        prefixes = (*self.image_prefixes, *self.container_prefixes)
        if any(not prefix or prefix.isspace() for prefix in prefixes):
            raise ValueError("Docker ownership prefixes must contain visible text")
        if self.receipt_stage == self.log_stage:
            raise ValueError("runtime receipts and logs require distinct stages")
        return self


class TartRuntimePolicy(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    kind: Literal["tart"]
    command: StrictStr
    timeout_seconds: PositiveInt
    receipt_stage: StrictStr
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
        if not self.command or any(character.isspace() for character in self.command):
            raise ValueError("Tart command must be one executable token")
        if not self.vm_prefixes or not self.base_images or not self.home:
            raise ValueError("Tart ownership, base images, and home must be non-empty")
        if self.receipt_stage == self.log_stage:
            raise ValueError("runtime receipts and logs require distinct stages")
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
