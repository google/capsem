"""A run log the gate could not finish writing never breaks the next gate.

Observed: a run died on ENOSPC mid-append, leaving one line of its own
`run.jsonl` torn -- a step event cut off partway, with the `run.end` written
after the disk freed glued onto the end of it. Every later gate on that
checkout then crashed in prefix export, because syncing the ledger decoded
every historical run and the one torn line raised.

A killed or starved writer is an ordinary input here, not an exotic one. The
readers keep every event that did survive, count what did not, and treat a
damaged run the way they treat one that never reached `close`: it is kept as a
directory to read, never as a clean measurement.
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import digestreport, runhistory, runledger, runs, testadmission
from capsem_builder.gate.context import Context
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.runlog import RunLog
from capsem_builder.gate.runlogschema import StepEnd
from capsem_builder.gate.timingratchet import TimingBoundary, enforce_current
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
RUN_ID = "20260927-140226-7a3208-focus-test"


@pytest.fixture
def checkout(tmp_path: Path) -> Path:
    (tmp_path / "config").mkdir()
    shutil.copy(PROJECT_ROOT / "config" / "gate.toml", tmp_path / "config" / "gate.toml")
    return tmp_path


def _event(kind: str, **fields: object) -> dict:
    return {"schema": CONFIG.runlog.event_schema, "ts": 1.0, "run_id": RUN_ID, "event": kind, **fields}


def _complete() -> list[dict]:
    return [
        _event(
            "run.start",
            command="candidate",
            argv=["candidate"],
            head="0" * 40,
            platform="Linux",
            machine="x86_64",
            cores=8,
            free_gb=1.0,
            gate_source="src",
            pycache="cache",
        ),
        _event("plan", steps=["build"], edges=[]),
        _event("step.end", step="build", status="ok", duration_ms=10.0, error=None),
        _event("run.end", status="ok", duration_ms=10.0, failures={}),
    ]


def _lines(events: list[dict]) -> list[bytes]:
    return [json.dumps(event).encode() for event in events]


def _write(root: Path, lines: list[bytes]) -> Path:
    target = root / CONFIG.runlog.root / RUN_ID
    target.mkdir(parents=True, exist_ok=True)
    (target / CONFIG.runlog.events).write_bytes(b"".join(line + b"\n" for line in lines))
    return target


def _enospc() -> list[bytes]:
    """The observed shape: a torn step event with the next event glued onto it."""
    lines = _lines(_complete())
    torn = lines[2][: len(lines[2]) // 2] + lines[3]
    return [lines[0], lines[1], torn]


# Every shape a writer that dies, runs out of disk, or is overwritten can leave.
HOSTILE = {
    "enospc-glued": (_enospc(), 2, 1),
    "torn-last-line": ([*_lines(_complete())[:3], _lines(_complete())[3][:17]], 3, 1),
    "torn-middle-line": (
        [*_lines(_complete())[:2], _lines(_complete())[2][:25], _lines(_complete())[3]],
        3,
        1,
    ),
    "non-utf8-bytes": ([*_lines(_complete())[:3], b'{"event": "\xff\xfe\xfa"}'], 3, 1),
    "json-but-not-an-object": (
        [*_lines(_complete()), b"[1, 2]", b"42", b'"run.end"', b"null"],
        4,
        4,
    ),
    "object-without-an-event": ([*_lines(_complete()), b'{"run_id": "x"}'], 4, 1),
    "empty-file": ([], 0, 0),
    "only-blank-lines": ([b"", b"   "], 0, 0),
}


@pytest.mark.parametrize(("lines", "kept", "torn"), HOSTILE.values(), ids=HOSTILE.keys())
def test_every_surviving_event_is_read_and_every_torn_line_counted(
    checkout: Path, lines: list[bytes], kept: int, torn: int
) -> None:
    directory = _write(checkout, lines)
    recovered = runhistory.recover(directory, CONFIG.runlog)
    assert (len(recovered.events), recovered.torn) == (kept, torn)
    assert all(isinstance(event.get("event"), str) for event in recovered.events)
    assert runhistory.read(directory, CONFIG.runlog) == recovered.events


@pytest.mark.parametrize(("lines", "kept", "torn"), HOSTILE.values(), ids=HOSTILE.keys())
def test_one_damaged_run_never_breaks_ledger_sync_or_the_digest(
    checkout: Path, lines: list[bytes], kept: int, torn: int
) -> None:
    """The reported crash: `runtransfer.export` -> `runledger.sync` raised."""
    del kept
    config = gate_config.load(checkout)
    _write(checkout, lines)
    runledger.sync(config, config.runlog)
    digestreport.write(config)
    if torn:
        assert runledger.rows(config) == [], "a damaged run is not a clean measurement"


def test_a_damaged_run_with_a_readable_end_is_still_not_a_measurement(checkout: Path) -> None:
    """The end survived but a step did not: its median contribution would be a lie."""
    config = gate_config.load(checkout)
    lines = _lines(_complete())
    _write(checkout, [lines[0], lines[1], lines[2][:30], lines[3]])
    assert runledger.sync(config, config.runlog) == 0
    assert runledger.rows(config) == []


def test_the_same_run_intact_is_recorded(checkout: Path) -> None:
    """The control: the refusal above is about the damage, not the fixture."""
    config = gate_config.load(checkout)
    _write(checkout, _lines(_complete()))
    assert runledger.sync(config, config.runlog) == 1


def test_a_non_utf8_ledger_line_does_not_blank_the_history(checkout: Path) -> None:
    config = gate_config.load(checkout)
    _write(checkout, _lines(_complete()))
    runledger.sync(config, config.runlog)
    ledger = runledger.path(config)
    ledger.write_bytes(b'{"run_id": "\xff\xfe"}\n' + ledger.read_bytes())

    assert [row.run_id for row in runledger.rows(config)] == [RUN_ID]
    assert runledger.sync(config, config.runlog) == 0
    digestreport.write(config)


def test_rotation_reads_a_non_utf8_run_without_raising(checkout: Path) -> None:
    config = gate_config.load(checkout)
    directory = _write(checkout, [*_lines(_complete())[:2], b"\xff\xfe"])
    assert runhistory.finished(directory, config.runlog) is False
    runhistory.rotate(config)


def test_runs_list_and_show_name_the_damage(checkout: Path) -> None:
    config = gate_config.load(checkout)
    directory = _write(checkout, _enospc())
    runner = RecordingRunner(checkout)
    context = Context(runner, config)

    runs._list(context)
    runs._explain(context, directory)
    assert "DAMAGED" in runner.notes[0]
    assert "1 line" in runner.notes[1]


def test_a_run_with_nothing_readable_is_still_refused_by_show(checkout: Path) -> None:
    config = gate_config.load(checkout)
    directory = _write(checkout, [b"{torn"])
    with pytest.raises(GateError, match="no recorded events"):
        runs._explain(Context(RecordingRunner(checkout), config), directory)


def test_a_damaged_run_is_never_the_timing_baseline(checkout: Path) -> None:
    config = gate_config.load(checkout)
    invocation = ("capsem-gate", "candidate")
    boundary = TimingBoundary.QUALIFICATION
    shape = ("compile", boundary.value)
    edges = (("compile", boundary.value),)

    with RunLog.open(config, "candidate", argv=invocation) as baseline:
        baseline.shape(shape, edges)
        baseline.emit(StepEnd(step="compile", status="ok", duration_ms=1_000))
        baseline.emit(StepEnd(step=boundary.value, status="ok", duration_ms=1))
    with (baseline.directory / config.runlog.events).open("ab") as sink:
        sink.write(b'{"event": "step.end", "st\n')

    with RunLog.open(config, "candidate", argv=invocation) as current:
        current.shape(shape, edges)
        current.emit(StepEnd(step="compile", status="ok", duration_ms=1_600))
        assert enforce_current(config, current.directory, boundary) is None


def test_a_damaged_green_candidate_is_not_admission_evidence(checkout: Path) -> None:
    config = gate_config.load(checkout)
    lines = _lines(_complete())
    _write(checkout, [lines[0], lines[1], lines[2][:30], lines[3]])
    assert testadmission._working_tree_history(config) == (True, None)
