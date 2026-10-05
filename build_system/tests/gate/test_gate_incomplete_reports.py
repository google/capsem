"""An unfinished gate journal is evidence of interruption, never success."""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.context import Context
from capsem_builder.gate.locks import ExclusiveLock
from capsem_builder.gate.proc import Runner
from capsem_builder.gate.runhistory import read
from capsem_builder.gate.runledger import distill
from capsem_builder.gate.runs import RunsCommand, _list
from capsem_builder.gate.summary import write_summary
from capsem_builder.gate.timing import measure, report

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)
INCOMPLETE_RATIONALE = (
    "#312: a missing run.end cannot mean success, even when every recorded "
    "step passed; RELEASE.md requires completed exact-source qualification."
)


def _config(root: Path) -> gate_config.GateConfig:
    policy = CONFIG.locks.gate.model_copy(update={
        "path": str(root / "machine.lock"),
        "holder_record": str(root / "machine.holder"),
        "report_after_seconds": 0.01,
        "poll_interval_seconds": 0.01,
        "wait_timeout_seconds": 30,
    })
    return CONFIG.model_copy(update={
        "root": root,
        "locks": CONFIG.locks.model_copy(update={"gate": policy}),
    })


@pytest.mark.parametrize("steps", [[], [{
    "event": "step.end", "step": "built", "status": "ok", "duration_ms": 5,
}]])
def test_missing_terminal_event_is_incomplete_even_after_passing_steps(steps: list[dict]) -> None:
    timing = measure([{"event": "run.start"}, *steps])
    rendered = report(timing, command="test", settings=CONFIG.runlog, run_id="unfinished")

    assert timing.outcome == "incomplete", INCOMPLETE_RATIONALE
    assert rendered.splitlines()[0].endswith(" -- INCOMPLETE"), INCOMPLETE_RATIONALE
    assert timing.steps == ({"built": 5} if steps else {})


def test_terminal_event_without_outcome_cannot_supply_success() -> None:
    timing = measure([{"event": "run.end", "duration_ms": 5}])
    assert timing.outcome == "incomplete", INCOMPLETE_RATIONALE


@pytest.mark.parametrize("status", ["ok", "failed"])
def test_completed_outcomes_keep_their_recorded_verdict(status: str) -> None:
    timing = measure([{"event": "run.end", "status": status, "duration_ms": 7}])
    rendered = report(timing, command="test", settings=CONFIG.runlog, run_id="complete")
    assert timing.outcome == status
    assert rendered.splitlines()[0] == f"test -- 0.0s -- {status if status == 'ok' else 'FAILED'}"
    assert timing.total_ms == 7


LOCK_WAIT = """
    import argparse, sys
    from pathlib import Path
    from capsem_builder.gate import cancellation, config as gate_config
    from capsem_builder.gate.lifecycle import held
    from capsem_builder.gate.locks import ExclusiveLock
    from capsem_builder.gate.recording import Recorded

    config = gate_config.load(Path(sys.argv[1]))
    root = Path(sys.argv[2])
    policy = config.locks.gate.model_copy(update={
        "path": str(root / "machine.lock"),
        "holder_record": str(root / "machine.holder"),
        "report_after_seconds": 0.01,
        "poll_interval_seconds": 0.01,
        "wait_timeout_seconds": 30,
    })
    class Waiting(Recorded):
        name = "lock-wait"
    command = Waiting()
    command._config = config.model_copy(update={
        "root": root,
        "locks": config.locks.model_copy(update={"gate": policy}),
    })
    command._args = argparse.Namespace(timing=True)
    command._invocation = ("lock-wait",)
    try:
        with cancellation.unwind_sigterm(), command._recording():
            with held(ExclusiveLock.for_gate(command._config, purpose="waiting",
                      announce=lambda message: print(message, flush=True))):
                raise AssertionError("the held kernel lock was bypassed")
    except KeyboardInterrupt:
        raise SystemExit(130)
    except cancellation.Terminated as error:
        raise SystemExit(error.exit_status)
"""


@pytest.mark.parametrize("interruption", [signal.SIGINT, signal.SIGTERM, signal.SIGKILL])
def test_real_interrupted_lock_wait_never_reports_success(
    tmp_path: Path, interruption: signal.Signals, capsys: pytest.CaptureFixture[str],
) -> None:
    config = _config(tmp_path)
    holder = ExclusiveLock.for_gate(config, purpose="isolated test holder")
    holder.acquire()
    try:
        child = subprocess.Popen(
            [sys.executable, "-u", "-c", textwrap.dedent(LOCK_WAIT), str(ROOT), str(tmp_path)],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        try:
            assert child.stdout is not None
            assert child.stdout.readline().startswith("waiting: another gate holds it"), (
                "the child must actually contend on the held kernel lock before interruption"
            )
            child.send_signal(interruption)
            output, errors = child.communicate(timeout=10)
        finally:
            if child.poll() is None:
                child.kill()
            child.wait(timeout=10)
        assert child.returncode == (-signal.SIGKILL if interruption == signal.SIGKILL
                                    else 128 + interruption), errors
        directory = config.path(config.runlog.root) / config.runlog.latest_link
        events = read(directory, config.runlog)
        assert events[0]["event"] == "run.start"
        assert not any(event["event"].startswith("step.") for event in events)
        timing = measure(events)
        if interruption == signal.SIGKILL:
            assert [event["event"] for event in events] == ["run.start"]
            assert timing.outcome == "incomplete", INCOMPLETE_RATIONALE
            assert distill(events, config.runlog.ledger) is None
        else:
            assert timing.outcome == "failed"
            cause = "interrupted" if interruption == signal.SIGINT else "terminated by SIGTERM"
            assert timing.run_failures["lock-wait"] == f"cancelled: {cause}"
            assert " -- FAILED" in output
        assert " -- ok" not in output, INCOMPLETE_RATIONALE

        # All readers consume the same untouched events, including an older
        # start-only journal. Writing the summary must not manufacture an end.
        original = (directory / config.runlog.events).read_bytes()
        write_summary(directory, config.runlog, command="lock-wait", run_id=directory.name)
        summary = (directory / config.runlog.summary).read_text()
        context = Context(Runner(tmp_path), config)
        _list(context)
        listing = capsys.readouterr().err
        expected = "INCOMPLETE" if interruption == signal.SIGKILL else "FAILED"
        assert expected in summary
        assert expected in listing
        assert "  ok" not in listing, INCOMPLETE_RATIONALE
        assert (directory / config.runlog.events).read_bytes() == original
        command = RunsCommand(Runner(ROOT), argparse.Namespace())
        assert command._latest(context, failed=True) == directory.resolve()
        assert json.loads(Path(config.locks.gate.holder_record).read_text())["pid"] == os.getpid()
    finally:
        holder.release()
