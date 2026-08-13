"""The ledger records each run once, and the digest only claims what it measured.

Every number the digest prints is a median over runs, which makes two mistakes
possible that do not announce themselves: counting one run twice, and treating
a step that did not run as a step that ran instantly. Both make the gate look
better than it is, so both are asserted here rather than left to inspection.
"""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate import runledger
from capsem.gate.rundigest import advice, analyse
from capsem.gate.runledger import LedgerRow, StepRow
from capsem.gate.runlog import RunLog
from capsem.gate.runlogschema import PlanShape, StepEnd

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
DIGEST = CONFIG.runlog.digest


def _events(
    run_id: str,
    *,
    command: str = "candidate",
    steps: tuple[tuple[str, float, str], ...] = (("build", 100.0, "ok"),),
    edges: tuple[tuple[str, str], ...] = (),
    ended: bool = True,
) -> list[dict]:
    events: list[dict] = [
        {
            "event": "run.start",
            "run_id": run_id,
            "command": command,
            "argv": [command],
            "head": "0" * 40,
            "platform": "Linux",
            "machine": "x86_64",
            "cores": 8,
            "free_gb": 100.0,
            "gate_source": "src",
            "pycache": "cache",
        },
        {
            "event": "plan",
            "run_id": run_id,
            "steps": [label for label, _ms, _status in steps],
            "edges": [list(edge) for edge in edges],
        },
    ]
    events += [
        {
            "event": "step.end",
            "run_id": run_id,
            "step": label,
            "duration_ms": spent,
            "status": status,
            "error": None,
        }
        for label, spent, status in steps
    ]
    if ended:
        events.append(
            {
                "event": "run.end",
                "run_id": run_id,
                "status": "ok",
                "duration_ms": sum(spent for _l, spent, _s in steps),
                "failures": {},
            }
        )
    return events


@pytest.fixture
def checkout(tmp_path: Path) -> Path:
    """A tree the ledger can be written into, with the real config."""
    (tmp_path / "config").mkdir()
    shutil.copy(PROJECT_ROOT / "config" / "gate.toml", tmp_path / "config" / "gate.toml")
    return tmp_path


def _record(root: Path, events: list[dict], *, directory: str | None = None) -> Path:
    run_id = events[0]["run_id"]
    target = root / CONFIG.runlog.root / (directory or run_id)
    target.mkdir(parents=True, exist_ok=True)
    import json

    (target / CONFIG.runlog.events).write_text(
        "\n".join(json.dumps(event) for event in events) + "\n", encoding="utf-8"
    )
    return target


def _row(
    run_id: str,
    *,
    command: str = "candidate",
    identity: str = "same",
    steps: dict[str, tuple[float, str]],
    critical: tuple[str, ...] = (),
    status: str = "ok",
) -> LedgerRow:
    return LedgerRow(
        row_schema="test",
        run_id=run_id,
        command=command,
        head="0" * 40,
        status=status,
        total_ms=sum(spent for spent, _s in steps.values()),
        identity=identity,
        critical_path=critical,
        steps={
            label: StepRow(duration_ms=spent, status=state)
            for label, (spent, state) in steps.items()
        },
    )


# -- the ledger -------------------------------------------------------------


def test_an_unfinished_run_is_not_recorded(checkout: Path) -> None:
    """A killed run has a truncated duration; a median must never see it."""
    config = gate_config.load(checkout)
    _record(checkout, _events("20260101-000000-aaaaaa-candidate", ended=False))
    assert runledger.sync(config, config.runlog) == 0
    assert runledger.rows(config) == []


def test_one_run_in_two_directories_is_recorded_once(checkout: Path) -> None:
    """Observed in a live tree: two directories holding one run's events.

    Keyed on the directory name it was counted twice, which biases every
    statistic computed from it and does so invisibly.
    """
    config = gate_config.load(checkout)
    events = _events("20260101-000000-aaaaaa-candidate")
    _record(checkout, events)
    _record(checkout, events, directory="20251231-235959-bbbbbb-something-else")

    runledger.sync(config, config.runlog)
    assert [row.run_id for row in runledger.rows(config)] == [
        "20260101-000000-aaaaaa-candidate"
    ]


def test_syncing_twice_adds_nothing(checkout: Path) -> None:
    config = gate_config.load(checkout)
    _record(checkout, _events("20260101-000000-aaaaaa-candidate"))
    assert runledger.sync(config, config.runlog) == 1
    assert runledger.sync(config, config.runlog) == 0


def test_the_ledger_is_trimmed_to_its_bound(checkout: Path) -> None:
    """Kept forever means bounded, or it becomes the disk-full it protects.

    Pre-filled rather than driven through two thousand run directories: the
    bound is a property of `append`, and paying twelve seconds to restate it
    is how a fast suite stops being fast.
    """
    config = gate_config.load(checkout)
    bound = config.runlog.ledger.keep_rows
    ledger = runledger.path(config)
    ledger.parent.mkdir(parents=True, exist_ok=True)
    ledger.write_text(
        "\n".join(
            _row(f"20250101-{index:06d}-old-candidate", steps={"build": (1.0, "ok")})
            .model_dump_json()
            for index in range(bound + 5)
        )
        + "\n",
        encoding="utf-8",
    )

    _record(checkout, _events("20260101-000000-aaaaaa-candidate"))
    assert runledger.sync(config, config.runlog) == 1
    recorded = runledger.rows(config)
    assert len(recorded) == bound
    assert recorded[0].run_id == "20260101-000000-aaaaaa-candidate", (
        "the newest run must survive the trim; dropping it would make the "
        "bound delete exactly the row the digest is about to read"
    )


def test_a_row_survives_beside_one_it_cannot_read(checkout: Path) -> None:
    """The ledger spans schema changes; one bad line must not blank the rest."""
    config = gate_config.load(checkout)
    _record(checkout, _events("20260101-000000-aaaaaa-candidate"))
    runledger.sync(config, config.runlog)

    ledger = runledger.path(config)
    ledger.write_text("{not json at all\n" + ledger.read_text(encoding="utf-8"), encoding="utf-8")
    assert len(runledger.rows(config)) == 1


def test_closing_a_run_records_it_and_rewrites_the_digest(checkout: Path) -> None:
    """The whole path, through the real writer.

    Asserted end to end because the two halves are wired in different places
    -- `RunLog.close` for the row, the fast phase for the rebuild -- and a
    contract that only checks the pieces passes happily while the run that
    just finished is missing from its own digest.
    """
    config = gate_config.load(checkout)
    with RunLog.open(config, "candidate", argv=("candidate",)) as log:
        log.emit(PlanShape(steps=("build",), edges=()))
        log.emit(StepEnd(step="build", status="ok", duration_ms=5.0))

    recorded = runledger.rows(config)
    assert [row.command for row in recorded] == ["candidate"]
    assert recorded[0].measured("build") == 5.0

    digest = checkout / config.runlog.digest.path
    assert digest.is_file(), "closing a run must leave a readable digest behind"
    assert recorded[0].run_id in digest.read_text(encoding="utf-8")


def test_a_bookkeeping_failure_never_replaces_the_real_one(checkout: Path) -> None:
    """`close` runs on the failure path, where the failure is the report.

    The lifecycle rule is that a primary error survives cleanup. A ledger that
    could raise from `close` would swallow the exception somebody actually
    needs, and would do it precisely on the runs that matter most.
    """
    config = gate_config.load(checkout)
    ledger = runledger.path(config)
    with (
        pytest.raises(RuntimeError, match="the real failure"),
        RunLog.open(config, "candidate", argv=("candidate",)) as log,
    ):
        log.emit(PlanShape(steps=("build",), edges=()))
        # A directory where the ledger file belongs, so recording cannot work.
        ledger.parent.mkdir(parents=True, exist_ok=True)
        ledger.mkdir(exist_ok=True)
        raise RuntimeError("the real failure")


# -- what counts as a measurement -------------------------------------------


@pytest.mark.parametrize("status", ["skipped", "carried", "failed"])
def test_a_step_that_did_not_do_the_work_is_not_a_measurement(status: str) -> None:
    row = _row("r", steps={"build": (0.5, status)})
    assert row.measured("build") is None


def test_the_median_ignores_runs_where_the_step_was_skipped() -> None:
    """Skips record a near-zero duration, and a median over them says free.

    Without this the digest reports a build that never ran as the fastest one
    on record, and then flags the next real build as a regression against it.
    """
    history = [
        _row("r5", steps={"build": (100.0, "ok")}, critical=("build",)),
        *[_row(f"r{n}", steps={"build": (0.4, "skipped")}, critical=("build",)) for n in (4, 3)],
        *[_row(f"r{n}", steps={"build": (95.0, "ok")}, critical=("build",)) for n in (2, 1)],
    ]
    analysis = analyse(history, DIGEST)
    assert analysis.regressions == [], (
        "100ms against a 95ms median is not a regression; a median dragged to "
        "zero by two skips would have made it one"
    )


def test_failures_are_counted_across_commands() -> None:
    """A red step is red whatever command ran it.

    Scoped to the latest run's command, the most useful finding in the tree --
    a step that failed repeatedly during candidate runs -- disappeared the
    moment somebody ran a fast test afterwards.
    """
    history = [
        _row("r3", command="test-fast", identity="fast", steps={"lint": (1.0, "ok")}),
        *[
            _row(f"r{n}", command="candidate", steps={"contracts": (1.0, "failed")})
            for n in (2, 1)
        ],
    ]
    analysis = analyse(history, DIGEST)
    assert [thrash.label for thrash in analysis.thrash] == ["contracts"]


# -- what the digest is willing to say --------------------------------------


def test_without_a_comparable_run_no_duration_is_compared() -> None:
    """Absence of evidence must not be printed as evidence of absence."""
    history = [
        _row("r2", identity="new-shape", steps={"build": (900.0, "ok")}, critical=("build",)),
        _row("r1", identity="old-shape", steps={"build": (10.0, "ok")}, critical=("build",)),
    ]
    analysis = analyse(history, DIGEST)
    assert analysis.baseline == []
    assert analysis.regressions == []
    assert "no durations were compared" in " ".join(advice(analysis, DIGEST))


def test_a_regression_names_the_measurement_it_came_from() -> None:
    history = [
        _row("r4", steps={"build": (400.0, "ok")}, critical=("build",)),
        *[_row(f"r{n}", steps={"build": (100.0, "ok")}, critical=("build",)) for n in (3, 2, 1)],
    ]
    analysis = analyse(history, DIGEST)
    assert [trend.label for trend in analysis.regressions] == ["build"]
    assert any("4.0x" in line for line in advice(analysis, DIGEST))


def test_following_one_step_is_not_scoped_to_the_latest_command() -> None:
    """`runs trend --step` must find the step wherever it ran.

    Scoped to the latest run's comparable window it printed nothing whenever
    the newest run happened to be a different command -- which reads exactly
    like a step that does not exist, and sends the reader looking for a typo.
    """
    history = [
        _row("r3", command="test-fast", identity="fast", steps={"lint": (1.0, "ok")}),
        *[
            _row(f"r{n}", command="candidate", steps={"contracts": (10.0, "ok")})
            for n in (2, 1)
        ],
    ]
    assert [row.run_id for row in runledger.containing(history, "contracts", 10)] == ["r2", "r1"]
    assert runledger.containing(history, "no-such-step", 10) == []


def test_a_long_step_off_the_critical_path_is_not_a_hotspot() -> None:
    """Residency, not duration -- the ranked-by-duration list sends people to
    optimize work that was never in the way."""
    history = [
        _row(
            f"r{n}",
            steps={"slow-but-parallel": (900.0, "ok"), "on-the-path": (300.0, "ok")},
            critical=("on-the-path",),
        )
        for n in range(4)
    ]
    analysis = analyse(history, DIGEST)
    assert [hotspot.label for hotspot in analysis.hotspots] == ["on-the-path"]
