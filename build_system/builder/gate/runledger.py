"""One distilled row per finished run, kept after its directory is gone.

Rotation is right to be aggressive -- a run directory is megabytes and the
crashed ones are the ones worth keeping -- but it means the gate could only
ever answer questions about its own recent past. The three questions people
actually ask are all longitudinal: is this getting slower, does that keep
failing, did the change I made help. Each needs the runs rotation has already
deleted.

So a run leaves two things behind. The directory, which is complete and
short-lived, and a row here, which is small and permanent. The row holds what a
trend needs and nothing else: who ran, on what class of machine, against which
plan, and what each step cost.

Comparability is defined once, in `identity`. Durations from two runs mean
nothing together unless they measured the same work on the same kind of
machine, and every consumer that has ever needed that rule needs the same one.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import TypeVar

from .config import GateConfig
from .digestschema import LedgerConfig
from .filesystem import write_text
from .harnessschema import RunLogConfig
from .runhistory import history_locked, read, runs
from .runledgerschema import LedgerRow, StepRow
from .runlogschema import OK, PlanShape, RunEnd, RunStart
from .timing import measure

Model = TypeVar("Model", RunStart, PlanShape, RunEnd)


def identity(start: RunStart, shape: PlanShape) -> tuple:
    """The fields that make two runs' elapsed times comparable.

    One definition, because there is one rule. The release ratchet asks the
    same question about two run logs and the digest asks it about two ledger
    rows; when each spelled it out separately they were free to disagree about
    a field, and the disagreement would surface as a release refused or
    allowed for a reason nobody could locate.
    """
    argv = start.argv
    if start.source_commit is not None and argv[-1:] == (start.source_commit,):
        argv = argv[:-1]
    return (
        start.command,
        argv,
        start.platform,
        start.machine,
        start.cores,
        shape.steps,
        shape.edges,
    )


def identity_digest(start: RunStart, shape: PlanShape) -> str:
    """`identity` as a short stable string, for a row that must stay small."""
    encoded = json.dumps(identity(start, shape), sort_keys=True, default=list)
    return hashlib.blake2b(encoded.encode(), digest_size=16).hexdigest()


def one_event(events: list[dict], model: type[Model]) -> Model | None:
    """The single event of this kind, or nothing if it is not there exactly once.

    Nothing rather than raising. A truncated log is an ordinary case -- a
    killed gate leaves whatever it managed to write -- and every reader here
    would rather skip an unusable run than refuse to read the history.
    """
    kind = model.model_fields["event"].default
    matches = [event for event in events if event.get("event") == kind]
    if len(matches) != 1:
        return None
    payload = {key: matches[0][key] for key in model.model_fields if key in matches[0]}
    try:
        return model.model_validate(payload)
    except ValueError:
        # A killed writer can leave a syntactically valid but incomplete
        # terminal event. It is no more ledger evidence than a missing one.
        return None


def distill(events: list[dict], settings: LedgerConfig) -> LedgerRow | None:
    """A finished run reduced to its row, or nothing if it never finished.

    A run with no `run.end` crashed, and rotation deliberately keeps those --
    the directory is the artifact there. Writing a row for one would put a
    truncated duration into a median.
    """
    start = one_event(events, RunStart)
    shape = one_event(events, PlanShape)
    ended = one_event(events, RunEnd)
    if start is None or shape is None or ended is None:
        return None

    timing = measure(events)
    return LedgerRow(
        row_schema=settings.row_schema,
        run_id=_run_id(events),
        command=start.command,
        head=start.head,
        status=timing.outcome,
        total_ms=ended.duration_ms,
        identity=identity_digest(start, shape),
        critical_path=tuple(timing.critical_path),
        steps={
            label: StepRow(
                duration_ms=spent,
                status=timing.status.get(label, OK),
                resource_ms=timing.resource_waits.get(label, 0.0),
                dependency_ms=timing.dependency_waits.get(label, 0.0),
            )
            for label, spent in timing.steps.items()
        },
    )


def _run_id(events: list[dict]) -> str:
    return next((event["run_id"] for event in events if "run_id" in event), "")


def path(config: GateConfig) -> Path:
    return config.path(config.runlog.ledger.path)


def rows(config: GateConfig) -> list[LedgerRow]:
    """Every recorded row, newest first.

    A row that no longer validates is dropped rather than raised on. The
    ledger spans schema changes by construction -- that is what keeping it
    forever means -- and a reader that refuses the whole file because its
    oldest line is a version behind has thrown away the history it exists to
    protect.
    """
    source = path(config)
    if not source.is_file():
        return []
    kept: list[LedgerRow] = []
    for line in source.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            row = LedgerRow.model_validate_json(line)
        except ValueError:
            continue
        if row.row_schema == config.runlog.ledger.row_schema:
            kept.append(row)
    # Run ids begin with a sortable UTC timestamp. Do not trust file order:
    # older prefix exports once copied their private ledger over this one and
    # could leave the newest row at the front. Every consumer wants newest
    # first regardless of how legacy bytes happened to be arranged.
    return sorted(kept, key=lambda row: row.run_id, reverse=True)


def append(config: GateConfig, directory: Path, settings: RunLogConfig) -> LedgerRow | None:
    """Record a finished run, and trim the file to its bound.

    Under the history lock, which is the same short hold that protects
    allocation and rotation. Two gates finishing together would otherwise
    interleave partial lines into one file, and a corrupt row is a run whose
    history is silently absent.

    A run is its recorded id, never its directory name. Those disagree in
    practice: two directories in a live tree were found holding one run's
    complete events, summary included, so keying on the name counted that run
    twice. Every statistic here is a median over runs, and a duplicate is the
    one kind of corruption that makes the numbers *more* confident rather than
    obviously broken.
    """
    row = distill(read(directory, settings), settings.ledger)
    if row is None:
        return None

    target = path(config)
    with history_locked(config):
        if _already_recorded(target, row.run_id, settings.ledger.row_schema):
            return None
        existing = target.read_text(encoding="utf-8").splitlines() if target.is_file() else []
        kept = [line for line in existing if line.strip() and _raw_run_id(line) != row.run_id]
        kept.append(row.model_dump_json())
        kept.sort(key=lambda line: _raw_run_id(line) or "")
        kept = kept[-settings.ledger.keep_rows :]
        write_text(target, "\n".join(kept) + "\n")
    return row


def _raw_run_id(line: str) -> str | None:
    """The claimed run id in a row, including one that does not validate."""
    try:
        payload = json.loads(line)
    except ValueError:
        return None
    return payload.get("run_id") if isinstance(payload, dict) else None


def _already_recorded(target: Path, run_id: str, row_schema: str) -> bool:
    """Whether a current, valid row for this run exists under the lock."""
    if not target.is_file():
        return False
    for line in target.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            row = LedgerRow.model_validate_json(line)
            if row.run_id == run_id and row.row_schema == row_schema:
                return True
        except ValueError:
            continue
    return False


def sync(config: GateConfig, settings: RunLogConfig) -> int:
    """Ingest every finished run directory the ledger does not already hold.

    Two cases need this and neither is exotic. On the day the ledger is added
    there is a tree full of history and an empty file, and a trend feature that
    says nothing for its first ten runs is one nobody keeps. And a run killed
    hard enough never reaches `close`, so its row is only ever written by
    somebody coming back for it.

    Cheap after the first pass: directories already recorded are matched by
    name, so the steady-state cost is one listing. Returns how many were added.
    """
    known = {row.run_id for row in rows(config)}
    added = 0
    # Oldest first, so the file stays in the order the runs happened and the
    # bound in `append` drops the oldest rather than an arbitrary row.
    for directory in reversed(runs(config)):
        if directory.name in known:
            continue
        if append(config, directory, settings) is not None:
            added += 1
    return added


def containing(history: list[LedgerRow], label: str, limit: int) -> list[LedgerRow]:
    """Recent rows that recorded this step, whatever command ran it.

    Deliberately not scoped to the latest run's comparable window. A hotspot
    worth following is usually in a different command from whatever ran last,
    and scoping it that way printed nothing -- which reads exactly like a step
    that does not exist.
    """
    return [row for row in history if label in row.steps][:limit]


def comparable_to(row: LedgerRow, history: list[LedgerRow], limit: int) -> list[LedgerRow]:
    """The most recent rows measuring the same work on the same host class."""
    return [
        other for other in history if other.identity == row.identity and other.run_id != row.run_id
    ][:limit]
