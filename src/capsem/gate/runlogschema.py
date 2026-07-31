"""What a run log line contains, one event type at a time.

Validated on the way out rather than trusted, for the same reason
`config/gate.toml` is: a log that anything may append to drifts into a shape
nothing can read back, and the first time anyone notices is when they need it
to diagnose a failure.

The envelope -- schema, timestamp, run id -- is added by the writer, so it
cannot be spelled differently by different events. These models are the
payloads.
"""

from __future__ import annotations

from typing import Literal

from pydantic import Field

from .configschema import Strict


class Payload(Strict):
    """One event's own fields."""


class RunStart(Payload):
    """Enough to tell whether two runs are comparable."""

    event: Literal["run.start"] = "run.start"
    command: str
    argv: tuple[str, ...]
    head: str
    platform: str
    machine: str
    cores: int
    free_gb: float


class PlanShape(Payload):
    """The graph, recorded so a finished run can still be explained.

    Without it a stored run has durations but no edges, and the critical path
    -- the only number worth acting on -- could be computed while the run was
    in memory and never again.
    """

    event: Literal["plan"] = "plan"
    steps: tuple[str, ...]
    edges: tuple[tuple[str, str], ...]
    """`(before, after)` pairs, in the order the plan declared them."""


class StepStart(Payload):
    event: Literal["step.start"] = "step.start"
    step: str
    actions: int
    contends: tuple[str, ...] = ()


class ActionRun(Payload):
    """One primitive, and what it actually was.

    `render` rather than a name: "run" as a label says nothing, and the whole
    reason actions can describe themselves is so this line is readable.
    """

    event: Literal["action"] = "action"
    step: str
    action: str
    render: str
    duration_ms: float
    status: str


class Exec(Payload):
    """One subprocess, recorded at the funnel every invocation passes through."""

    event: Literal["exec"] = "exec"
    step: str
    argv: tuple[str, ...]
    cwd: str
    env: dict[str, str]
    """Only what this command added.

    Never the inherited environment: a run log is a file people attach to bug
    reports, and the ambient environment of a release machine holds tokens.
    """

    exit: int
    duration_ms: float


class Artifact(Payload):
    """Bytes this run produced, so the question survives the tree."""

    event: Literal["artifact"] = "artifact"
    step: str
    path: str
    size: int
    digest: str


class Note(Payload):
    event: Literal["note"] = "note"
    step: str
    message: str


class StepEnd(Payload):
    event: Literal["step.end"] = "step.end"
    step: str
    status: str
    duration_ms: float
    error: str | None = None


class RunEnd(Payload):
    """The summary a person reads first."""

    event: Literal["run.end"] = "run.end"
    status: str
    duration_ms: float
    failures: dict[str, str] = Field(default_factory=dict)
    skipped: tuple[str, ...] = ()
    critical_path: tuple[str, ...] = ()
    footprint_gb: float = 0.0


#: Every payload, by the value of its `event` field. Used to read a log back
#: and to prove, in a test, that nothing writes a line no model describes.
PAYLOADS: dict[str, type[Payload]] = {
    model.model_fields["event"].default: model
    for model in (
        RunStart,
        PlanShape,
        StepStart,
        ActionRun,
        Exec,
        Artifact,
        Note,
        StepEnd,
        RunEnd,
    )
}
