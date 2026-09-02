"""Append-only warm/cold observations for policy-owned tool stages."""

from __future__ import annotations

import json
import os
import time
from enum import StrEnum
from pathlib import Path
from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, StrictInt

from .paths import CachePaths


class ReuseScope(StrEnum):
    """Whether bytes belong to one exact generation or a shared pool."""

    GENERATION = "generation"
    SHARED = "shared"


class CacheTemperature(StrEnum):
    """Observed pre-run availability, without claiming an eventual tool hit."""

    COLD = "cold"
    WARM = "warm"


class CacheUse(BaseModel):
    """One tool's view of its stage before doing work."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    schema_id: Literal["capsem.cache-use.v2"] = "capsem.cache-use.v2"
    timestamp_ns: Annotated[StrictInt, Field(ge=0)]
    stage_id: str
    tool: str
    key: str
    scope: ReuseScope
    temperature: CacheTemperature
    observed_bytes: Annotated[StrictInt, Field(ge=0)] | None = None


def _populated(path: Path, ignored_names: tuple[str, ...]) -> bool:
    if path.is_symlink():
        return False
    if path.is_file():
        return path.stat().st_size > 0
    if not path.is_dir():
        return False
    return any(child.name not in ignored_names for child in path.iterdir())


def record_use(
    paths: CachePaths,
    stage_id: str,
    *,
    tool: str,
    key: str,
    scope: ReuseScope,
    observed_bytes: int | None = None,
    probe: Path | None = None,
    ignored_names: tuple[str, ...] = (),
) -> CacheUse:
    """Record pre-run cache temperature without recursively measuring the stage."""
    stage = paths.stage(stage_id)
    subject = stage if probe is None else probe.absolute()
    if subject != stage and stage not in subject.parents:
        raise ValueError(f"cache telemetry probe {subject} is outside stage {stage}")
    warm = observed_bytes > 0 if observed_bytes is not None else _populated(subject, ignored_names)
    event = CacheUse(
        timestamp_ns=time.time_ns(),
        stage_id=stage_id,
        tool=tool,
        key=key,
        scope=scope,
        temperature=CacheTemperature.WARM if warm else CacheTemperature.COLD,
        observed_bytes=observed_bytes,
    )
    journal = paths.stage("state") / "usage.jsonl"
    journal.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(journal, os.O_APPEND | os.O_CREAT | os.O_WRONLY, 0o600)
    try:
        os.write(descriptor, (json.dumps(event.model_dump(mode="json")) + "\n").encode())
    finally:
        os.close(descriptor)
    return event
