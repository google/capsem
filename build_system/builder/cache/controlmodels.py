"""Strict policy models for cache control that crosses process boundaries."""

from __future__ import annotations

from typing import Annotated

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    StrictInt,
    StrictStr,
    field_validator,
    model_validator,
)

PositiveInt = Annotated[StrictInt, Field(gt=0)]
NonNegativeInt = Annotated[StrictInt, Field(ge=0)]


def _visible(value: str, *, field: str) -> str:
    if not value.strip():
        raise ValueError(f"{field} must contain visible text")
    return value


class CapacityRail(BaseModel):
    """Free-space floor and reusable BuildKit allowance for one build rail."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    minimum_free_bytes: PositiveInt
    build_cache_keep_bytes: PositiveInt
    reclaim_headroom_bytes: NonNegativeInt
    reclaim_attempts: PositiveInt


class ImageCachePolicy(BaseModel):
    """Retention for one content-keyed Docker image repository."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    repository: StrictStr
    keep_previous: NonNegativeInt = 0
    maximum_count: PositiveInt | None = None
    maximum_age_hours: PositiveInt | None = None
    maximum_bytes: PositiveInt | None = None

    @field_validator("repository")
    @classmethod
    def repository_is_one_name(cls, value: str) -> str:
        _visible(value, field="image repository")
        if any(character.isspace() for character in value) or "@" in value:
            raise ValueError("image repository must be one unpinned Docker name")
        return value.rstrip(":")

    @model_validator(mode="after")
    def bounds_are_complete(self) -> ImageCachePolicy:
        bounds = (self.maximum_count, self.maximum_age_hours, self.maximum_bytes)
        if any(value is not None for value in bounds) and not all(
            value is not None for value in bounds
        ):
            raise ValueError("image count, age, and byte bounds must be declared together")
        return self

    @property
    def maximum_age_seconds(self) -> int:
        if self.maximum_age_hours is None:
            raise ValueError(f"image cache {self.repository!r} has no receipt age bound")
        return self.maximum_age_hours * 3600


class ReleaseBoundary(BaseModel):
    """Exact working image tags released after their final consumer."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    rail: StrictStr
    images: tuple[StrictStr, ...]

    @field_validator("images", mode="before")
    @classmethod
    def images_are_frozen(cls, value: object) -> object:
        return tuple(value) if isinstance(value, list) else value

    @model_validator(mode="after")
    def values_are_canonical(self) -> ReleaseBoundary:
        _visible(self.rail, field="release rail")
        if not self.images or len(self.images) != len(set(self.images)):
            raise ValueError("release boundary images must be non-empty and unique")
        if any(not image.strip() or any(char.isspace() for char in image) for image in self.images):
            raise ValueError("release boundary images must be exact Docker references")
        return self


class DockerControlPolicy(BaseModel):
    """Capacity, repository, and lifetime policy for the Docker runtime."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    runtime_id: StrictStr
    capacity_probe_image: StrictStr
    minimum_disk_bytes: PositiveInt
    recommended_disk_bytes: PositiveInt
    rails: dict[StrictStr, CapacityRail]
    images: dict[StrictStr, ImageCachePolicy]
    releases: dict[StrictStr, ReleaseBoundary]

    @field_validator("capacity_probe_image")
    @classmethod
    def probe_image_is_immutable(cls, value: str) -> str:
        _visible(value, field="Docker capacity probe image")
        name, separator, digest = value.rpartition("@sha256:")
        if (
            not name
            or not separator
            or len(digest) != 64
            or any(char not in "0123456789abcdef" for char in digest)
        ):
            raise ValueError("Docker capacity probe image must be pinned by SHA-256 digest")
        return value

    @model_validator(mode="after")
    def policy_is_connected(self) -> DockerControlPolicy:
        _visible(self.runtime_id, field="Docker runtime id")
        if self.recommended_disk_bytes < self.minimum_disk_bytes:
            raise ValueError("recommended Docker disk bytes cannot be below the minimum")
        if not self.rails or not self.images:
            raise ValueError("Docker control requires capacity rails and image repositories")
        unknown = sorted({release.rail for release in self.releases.values()} - set(self.rails))
        if unknown:
            raise ValueError(f"release boundaries reference unknown rails: {', '.join(unknown)}")
        repositories = [image.repository for image in self.images.values()]
        if len(repositories) != len(set(repositories)):
            raise ValueError("Docker image repositories must be unique")
        return self

    def image_generation_limit(self, repository: str, *, default: int) -> int:
        """Return the configured total generations retained for a repository."""
        for image in self.images.values():
            if image.repository == repository and image.maximum_count is not None:
                return image.maximum_count
        return default


class FailureArtifactPolicy(BaseModel):
    """Bounded evidence retained when an expensive gate fails."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    stage: StrictStr
    minimum_count: NonNegativeInt
    maximum_count: PositiveInt
    maximum_age_hours: PositiveInt
    maximum_bytes: PositiveInt
    maximum_file_bytes: PositiveInt
    skip_names: tuple[StrictStr, ...]
    source_patterns: tuple[StrictStr, ...]

    @field_validator("skip_names", "source_patterns", mode="before")
    @classmethod
    def names_are_frozen(cls, value: object) -> object:
        return tuple(value) if isinstance(value, list) else value

    @model_validator(mode="after")
    def retention_is_possible(self) -> FailureArtifactPolicy:
        if self.minimum_count > self.maximum_count:
            raise ValueError("minimum failure evidence count cannot exceed maximum count")
        if len(self.skip_names) != len(set(self.skip_names)):
            raise ValueError("failure evidence skip names must be unique")
        if not self.source_patterns or len(self.source_patterns) != len(set(self.source_patterns)):
            raise ValueError("failure evidence source patterns must be non-empty and unique")
        if any(
            pattern.startswith("/") or ".." in pattern.split("/")
            for pattern in self.source_patterns
        ):
            raise ValueError("failure evidence sources must stay inside the cache root")
        return self


class CacheControlPolicy(BaseModel):
    """Non-filesystem cache policy owned by the same control plane."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    docker: DockerControlPolicy
    failure_artifacts: FailureArtifactPolicy


class DockerCapacitySnapshot(BaseModel):
    """Filesystem capacity observed from inside the Docker daemon."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    available: bool
    total_bytes: NonNegativeInt = 0
    used_bytes: NonNegativeInt = 0
    free_bytes: NonNegativeInt = 0
    error: StrictStr | None = None

    @model_validator(mode="after")
    def values_are_consistent(self) -> DockerCapacitySnapshot:
        if self.available and self.error is not None:
            raise ValueError("available Docker capacity cannot carry an error")
        if not self.available and not self.error:
            raise ValueError("unavailable Docker capacity must explain why")
        if self.available and self.used_bytes + self.free_bytes > self.total_bytes:
            raise ValueError("Docker used plus free bytes exceed total capacity")
        return self


class CapacityDecision(BaseModel):
    """Before/after proof for one Docker capacity rail."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    rail: StrictStr
    before: DockerCapacitySnapshot
    after: DockerCapacitySnapshot
    pruned: bool
    violations: tuple[StrictStr, ...]
