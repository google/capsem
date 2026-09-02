"""Live receipt authority for the reusable sealed install image."""

from __future__ import annotations

import hashlib
import time
from pathlib import Path
from typing import TYPE_CHECKING, Annotated, Literal

from pydantic import ConfigDict, Field, StringConstraints, ValidationError, model_validator

from .cachecontrol import CacheControl
from .config import GateConfig
from .configschema import Strict
from .docker import Docker
from .errors import GateError
from .filesystem import write_text
from .imageidentity import exact_image_id, require_exact_image, require_input_key
from .proc import Runner

if TYPE_CHECKING:
    from . import installbuilder, sourcecapture

CanonicalDigest = Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{64}$")]
ExactImageId = Annotated[str, StringConstraints(pattern=r"^sha256:[0-9a-f]{64}$")]
SCHEMA = "capsem.install-image-receipt.v2"


class InstallImageIdentity(Strict):
    """Exact inputs, runtime, product bytes, and cache lifetime."""

    model_config = ConfigDict(
        extra="forbid", frozen=True, validate_by_name=True, allow_inf_nan=False
    )

    schema_version: Literal["capsem.install-image-receipt.v2"] = Field(
        validation_alias="schema", serialization_alias="schema"
    )
    input_key: str = Field(min_length=1)
    input_digest: CanonicalDigest
    image_id: ExactImageId
    image_reference: str = Field(min_length=1)
    helper_input_key: str = Field(min_length=1)
    helper_image_id: ExactImageId
    source_digest: CanonicalDigest
    runtime_digest: CanonicalDigest
    platform: str = Field(min_length=1)
    image_size_bytes: int = Field(gt=0)
    created_at: float = Field(ge=0)
    last_used_at: float = Field(ge=0)

    @model_validator(mode="after")
    def _timestamps_are_ordered(self) -> InstallImageIdentity:
        if self.last_used_at < self.created_at:
            raise ValueError("last_used_at cannot precede created_at")
        return self


def digest(*values: str) -> str:
    found = hashlib.sha256()
    for value in values:
        found.update(value.encode("utf-8"))
        found.update(b"\0")
    return found.hexdigest()


def read(path: Path) -> InstallImageIdentity:
    try:
        return InstallImageIdentity.model_validate_json(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, ValidationError) as error:
        raise GateError(f"install image receipt {path} is missing or invalid") from error


def write(path: Path, identity: InstallImageIdentity) -> None:
    write_text(path, identity.model_dump_json(by_alias=True))


def validate(
    runner: Runner,
    config: GateConfig,
    *,
    path: Path,
    receipt: InstallImageIdentity,
    helper: installbuilder.InstallBuilderIdentity,
    source: sourcecapture.SourceSnapshot,
    tag: str,
    resource: str,
    input_key_label: str,
    touch: bool,
) -> InstallImageIdentity:
    """Revalidate every receipt edge against the live Docker product."""
    docker = Docker(runner)
    platform = config.host_arch().docker_platform
    runtime_digest = digest(docker.runtime_identity())
    input_digest = digest(tag, helper.input_key, helper.image_id, source.digest, runtime_digest)
    if receipt.source_digest != source.digest:
        raise GateError(
            f"install image receipt names source {receipt.source_digest}, expected {source.digest}"
        )
    if receipt.helper_input_key != helper.input_key or receipt.helper_image_id != helper.image_id:
        raise GateError("install image receipt no longer matches the exact dependency helper")
    if receipt.input_key != tag or receipt.input_digest != input_digest:
        raise GateError(f"install image receipt selects stale input key {receipt.input_key}")
    if receipt.runtime_digest != runtime_digest or receipt.platform != platform:
        raise GateError("install image receipt no longer matches the Docker runtime")
    policy = CacheControl(runner).image_policy(resource)
    now = time.time()
    if receipt.created_at > now or receipt.last_used_at > now:
        raise GateError("install image receipt has future-dated cache timestamps")
    if now - receipt.created_at > policy.maximum_age_seconds:
        raise GateError("install image receipt exceeded its configured cache age")
    if receipt.image_size_bytes > policy.max_size_bytes:
        raise GateError("install qualification image exceeds its configured byte bound")
    if not docker.image_exists(tag, platform=platform):
        raise GateError(f"install qualification image {tag} is missing")
    require_input_key(docker, tag, label=input_key_label, subject="install qualification image")
    image_id = exact_image_id(docker, tag, platform=platform, subject="install qualification image")
    if image_id != receipt.image_id:
        raise GateError(
            f"install qualification image {tag} moved: expected {receipt.image_id}, "
            f"found {image_id}"
        )
    require_exact_image(
        docker,
        receipt.image_reference,
        platform=platform,
        expected_id=receipt.image_id,
        subject="install qualification image build reference",
    )
    if docker.image_size(tag) != receipt.image_size_bytes:
        raise GateError("install qualification image byte size no longer matches its receipt")
    if touch:
        receipt = receipt.model_copy(update={"last_used_at": now})
        write(path, receipt)
    return receipt
