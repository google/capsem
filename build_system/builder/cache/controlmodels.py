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

from .contract import CacheContract, CacheScope, PruneStrategy

PositiveInt = Annotated[StrictInt, Field(gt=0)]
NonNegativeInt = Annotated[StrictInt, Field(ge=0)]


def _visible(value: str, *, field: str) -> str:
    if not value.strip():
        raise ValueError(f"{field} must contain visible text")
    return value


class ImageCachePolicy(CacheContract):
    """Retention for one content-keyed Docker image repository."""

    repository: StrictStr
    keep_previous: NonNegativeInt = 0
    maximum_count: PositiveInt | None = None
    maximum_age_hours: PositiveInt | None = None

    @field_validator("repository")
    @classmethod
    def repository_is_one_name(cls, value: str) -> str:
        _visible(value, field="image repository")
        if any(character.isspace() for character in value) or "@" in value:
            raise ValueError("image repository must be one unpinned Docker name")
        return value.rstrip(":")

    @model_validator(mode="after")
    def bounds_are_complete(self) -> ImageCachePolicy:
        if self.scope is not CacheScope.DOCKER:
            raise ValueError("Docker image caches require docker scope")
        if self.prune_strategy is not PruneStrategy.GENERATIONAL:
            raise ValueError("Docker image caches require generational pruning")
        return self

    @property
    def maximum_age_seconds(self) -> int:
        if self.maximum_age_hours is None:
            raise ValueError(f"image cache {self.repository!r} has no receipt age bound")
        return self.maximum_age_hours * 3600


class ReleaseBoundary(BaseModel):
    """Exact working image tags released after their final consumer."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    images: tuple[StrictStr, ...]

    @field_validator("images", mode="before")
    @classmethod
    def images_are_frozen(cls, value: object) -> object:
        return tuple(value) if isinstance(value, list) else value

    @model_validator(mode="after")
    def values_are_canonical(self) -> ReleaseBoundary:
        if not self.images or len(self.images) != len(set(self.images)):
            raise ValueError("release boundary images must be non-empty and unique")
        if any(not image.strip() or any(char.isspace() for char in image) for image in self.images):
            raise ValueError("release boundary images must be exact Docker references")
        return self


class DockerControlPolicy(BaseModel):
    """Repository and lifetime policy for the Docker runtime cache."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    runtime_id: StrictStr
    images: dict[StrictStr, ImageCachePolicy]
    releases: dict[StrictStr, ReleaseBoundary]

    @model_validator(mode="after")
    def policy_is_connected(self) -> DockerControlPolicy:
        _visible(self.runtime_id, field="Docker runtime id")
        if not self.images:
            raise ValueError("Docker control requires image repositories")
        repositories = [image.repository for image in self.images.values()]
        if len(repositories) != len(set(repositories)):
            raise ValueError("Docker image repositories must be unique")
        return self


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
