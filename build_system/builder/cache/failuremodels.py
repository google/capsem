"""Strict receipts for bounded failure-evidence capture."""

from __future__ import annotations

from enum import StrEnum
from pathlib import Path
from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, StrictStr, StringConstraints

CanonicalCommit = Annotated[StrictStr, StringConstraints(pattern=r"^[0-9a-f]{40}$")]
CanonicalRunId = Annotated[StrictStr, StringConstraints(pattern=r"^[A-Za-z0-9_.-]{1,160}$")]


class CollectionOutcome(StrEnum):
    COPIED = "copied"
    TRUNCATED = "truncated"
    ABSENT = "absent"
    UNREADABLE = "unreadable"


class CollectedEvidence(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    source: Path
    destination: Path | None
    outcome: CollectionOutcome


class FailureEvidenceManifest(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    schema_id: Literal["capsem.failure-evidence.v1"] = "capsem.failure-evidence.v1"
    created_ns: Annotated[int, Field(ge=0)]
    label: StrictStr
    run_id: CanonicalRunId | None = None
    source_commit: CanonicalCommit | None = None
    files: tuple[CollectedEvidence, ...]
