"""Strictly parse one archived exact-source attempt journal."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from .config import GateConfig
from .errors import GateError
from .filesystem import digest_of
from .runlogschema import (
    PAYLOADS,
    PlanShape,
    QualificationComplete,
    QualificationResume,
    QualificationReuse,
    QualificationRun,
    RunEnd,
    RunStart,
    StepEnd,
)


@dataclass(frozen=True)
class Attempt:
    reference: QualificationRun
    start: RunStart
    shape: PlanShape
    end: RunEnd
    steps: dict[str, str]
    resumed: QualificationResume | None
    complete: QualificationComplete | None
    reused: QualificationReuse | None


def reference(config: GateConfig, path: Path) -> QualificationRun:
    if path.is_symlink() or not path.is_file():
        raise GateError(f"qualification run log {path} is not a regular file")
    return QualificationRun(
        run_id=path.stem,
        run_log=str(path.absolute()),
        digest=digest_of(path, algorithm=config.runlog.artifact_digest),
    )


def _required(models: list[object], kind: type):
    selected = [model for model in models if isinstance(model, kind)]
    if len(selected) != 1:
        raise ValueError(f"qualification journal requires exactly one {kind.__name__}")
    return selected[0]


def _optional(models: list[object], kind: type):
    selected = [model for model in models if isinstance(model, kind)]
    if len(selected) > 1:
        raise ValueError(f"qualification journal permits at most one {kind.__name__}")
    return selected[0] if selected else None


def load(config: GateConfig, path: Path) -> Attempt | None:
    """Return one canonical attempt, or nothing for any malformed journal."""
    try:
        if path.is_symlink() or not path.is_file():
            return None
        models: list[object] = []
        run_id = path.stem
        for line in path.read_text(encoding="utf-8").splitlines():
            raw = json.loads(line)
            if (
                not isinstance(raw, dict)
                or raw.get("schema") != config.runlog.event_schema
                or raw.get("run_id") != run_id
                or not isinstance(raw.get("ts"), (int, float))
            ):
                return None
            model = PAYLOADS.get(raw.get("event"))
            if model is None:
                return None
            payload = {key: raw[key] for key in model.model_fields if key in raw}
            models.append(model.model_validate(payload))

        start = _required(models, RunStart)
        shape = _required(models, PlanShape)
        end = _required(models, RunEnd)
        if models[0] is not start or models[-1] is not end:
            return None

        endings = [model for model in models if isinstance(model, StepEnd)]
        if len({item.step for item in endings}) != len(endings):
            return None
        steps = {item.step: item.status for item in endings}
        if not set(steps) <= set(shape.steps):
            return None

        resumed = _optional(models, QualificationResume)
        complete = _optional(models, QualificationComplete)
        reused = _optional(models, QualificationReuse)
        first_step = min((models.index(item) for item in endings), default=len(models) - 1)
        if models.index(shape) >= first_step:
            return None
        if resumed is not None and models.index(resumed) >= first_step:
            return None
        if reused is not None and models.index(reused) >= first_step:
            return None
        if complete is not None and models.index(complete) <= max(
            (models.index(item) for item in endings), default=1
        ):
            return None
        return Attempt(reference(config, path), start, shape, end, steps, resumed, complete, reused)
    except (KeyError, OSError, TypeError, ValueError):
        return None
