"""Short-lived, content-keyed clean verdicts for live cache consumers."""

from __future__ import annotations

import secrets
import time
from pathlib import Path
from typing import Annotated, Literal

import blake3
from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    StrictInt,
    StrictStr,
    StringConstraints,
    ValidationError,
)

from .paths import CachePaths

SCHEMA = "capsem.clean-verdict.v1"
CanonicalDigest = Annotated[StrictStr, StringConstraints(pattern=r"^[0-9a-f]{64}$")]
Owner = Annotated[StrictStr, StringConstraints(pattern=r"^[a-z][a-z0-9-]*$")]


class CleanVerdict(BaseModel):
    """A successful live check bound to exact input and a bounded age."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    schema_version: Literal["capsem.clean-verdict.v1"] = SCHEMA
    owner: Owner
    subject_digest: CanonicalDigest
    checked_at_ns: Annotated[StrictInt, Field(ge=0)]
    messages: tuple[StrictStr, ...] = Field(min_length=1)


def subject_digest(payload: bytes) -> str:
    """Name the exact canonical request a live verdict answered."""
    return blake3.blake3(payload).hexdigest()


def _path(paths: CachePaths, stage_id: str, owner: str, digest: str) -> Path:
    validated_owner = CleanVerdict(
        owner=owner,
        subject_digest=digest,
        checked_at_ns=0,
        messages=("validation",),
    ).owner
    return paths.stage(stage_id) / validated_owner / f"{digest}.json"


def reusable(
    paths: CachePaths,
    *,
    stage_id: str,
    owner: str,
    digest: str,
    now_ns: int | None = None,
) -> CleanVerdict | None:
    """Return a matching unexpired receipt; malformed state is a cache miss."""
    target = _path(paths, stage_id, owner, digest)
    if target.is_symlink() or not target.is_file():
        return None
    try:
        verdict = CleanVerdict.model_validate_json(target.read_text(encoding="utf-8"))
    except (OSError, ValidationError):
        return None
    if verdict.owner != owner or verdict.subject_digest != digest:
        return None
    observed_ns = time.time_ns() if now_ns is None else now_ns
    maximum_age_ns = paths.policy.stages[stage_id].maximum_age_hours * 3_600_000_000_000
    if verdict.checked_at_ns > observed_ns or observed_ns - verdict.checked_at_ns > maximum_age_ns:
        return None
    return verdict


def record_clean(
    paths: CachePaths,
    *,
    stage_id: str,
    owner: str,
    digest: str,
    messages: tuple[str, ...],
    now_ns: int | None = None,
) -> CleanVerdict:
    """Atomically publish a clean verdict; failures and advisories never cache."""
    verdict = CleanVerdict(
        owner=owner,
        subject_digest=digest,
        checked_at_ns=time.time_ns() if now_ns is None else now_ns,
        messages=messages,
    )
    target = _path(paths, stage_id, owner, digest)
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_name(f".{target.name}.{secrets.token_hex(6)}")
    try:
        temporary.write_text(verdict.model_dump_json(indent=2) + "\n", encoding="utf-8")
        temporary.replace(target)
    finally:
        temporary.unlink(missing_ok=True)
    return verdict
