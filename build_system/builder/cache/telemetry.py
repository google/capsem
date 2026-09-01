"""Append-only warm/cold observations for policy-owned tool stages."""

from __future__ import annotations

import json
import os
import subprocess
import time
from enum import StrEnum
from pathlib import Path

from pydantic import BaseModel, ConfigDict, StrictInt

from .paths import CachePaths


class CacheOutcome(StrEnum):
    HIT = "hit"
    MISS = "miss"


class CacheUse(BaseModel):
    """One tool's view of its stage before doing work."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    timestamp_ns: StrictInt
    stage_id: str
    tool: str
    key: str
    outcome: CacheOutcome
    logical_bytes: StrictInt


def _size(path: Path) -> int:
    if not path.exists():
        return 0
    result = subprocess.run(("du", "-sk", str(path)), check=True, capture_output=True, text=True)
    return int(result.stdout.split()[0]) * 1024


def record_use(
    paths: CachePaths,
    stage_id: str,
    *,
    tool: str,
    key: str,
    logical_bytes: int | None = None,
    probe: Path | None = None,
) -> CacheUse:
    """Record whether a keyed invocation found reusable bytes in its stage."""
    stage = paths.stage(stage_id)
    subject = stage if probe is None else probe.absolute()
    if subject != stage and stage not in subject.parents:
        raise ValueError(f"cache telemetry probe {subject} is outside stage {stage}")
    size = _size(subject) if logical_bytes is None else logical_bytes
    event = CacheUse(
        timestamp_ns=time.time_ns(),
        stage_id=stage_id,
        tool=tool,
        key=key,
        outcome=CacheOutcome.HIT if size else CacheOutcome.MISS,
        logical_bytes=size,
    )
    journal = paths.stage("state") / "usage.jsonl"
    journal.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(journal, os.O_APPEND | os.O_CREAT | os.O_WRONLY, 0o600)
    try:
        os.write(descriptor, (json.dumps(event.model_dump(mode="json")) + "\n").encode())
    finally:
        os.close(descriptor)
    return event
