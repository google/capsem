"""Filesystem findings cross the watchdog thread before a release can proceed.

The watchdog callback is not the gate's controlling thread.  Raising there
only kills watchdog's dispatcher; it does not fail the plan that is about to
publish.  These tests keep the real thread boundary and the real run log in
the loop so a test double cannot accidentally turn that asynchronous failure
into a synchronous one.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.actions import Action, Launch, Run
from capsem_builder.gate.context import Context, NullJournal
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.execution import step
from capsem_builder.gate.funnel import GuardedRunner
from capsem_builder.gate.observation import Watch
from capsem_builder.gate.observing import observing
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.proc import Runner
from capsem_builder.gate.processgroup import StopPolicy
from capsem_builder.gate.runhistory import read
from capsem_builder.gate.runlog import RunLog

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
STOP_POLICY = StopPolicy(
    grace_seconds=CONFIG.execution.cancellation_grace_seconds,
    poll_seconds=CONFIG.execution.cancellation_poll_seconds,
)


def _checkout(tmp_path: Path):
    root = tmp_path / "checkout"
    source = root / "src"
    source.mkdir(parents=True)
    victim = source / "tracked.txt"
    victim.write_text("before\n", encoding="utf-8")
    for argv in (
        ("init", "--quiet"),
        ("config", "user.email", "gate@example.test"),
        ("config", "user.name", "Gate"),
        ("config", "commit.gpgsign", "false"),
        ("add", "src/tracked.txt"),
        ("commit", "--quiet", "-m", "tracked source"),
    ):
        subprocess.run(["git", *argv], cwd=root, check=True, capture_output=True)

    runlog = CONFIG.runlog.model_copy(update={"observed_roots": ("src",)})
    config = CONFIG.model_copy(update={"root": root, "runlog": runlog})
    return config, victim


def _mutate_then_restore(victim: Path, errors: Path) -> list[str]:
    """A child waits for durable detection before erasing the final-state trace."""
    script = """
import sys
import time
from pathlib import Path

victim, errors = map(Path, sys.argv[1:])
victim.write_text("during\\n", encoding="utf-8")
deadline = time.monotonic() + 5
while time.monotonic() < deadline:
    if errors.is_file() and "[source-tree]" in errors.read_text(encoding="utf-8"):
        victim.write_text("before\\n", encoding="utf-8")
        raise SystemExit(0)
    time.sleep(0.01)
raise SystemExit("watchdog never durably reported the tracked-source mutation")
"""
    return [sys.executable, "-c", script, str(victim), str(errors)]


def test_external_transient_source_fault_fails_the_plan_without_killing_watchdog(
    tmp_path: Path,
    capfd: pytest.CaptureFixture[str],
) -> None:
    """A subprocess mutate-and-revert is fatal at the next runner boundary.

    The bytes match again before the command exits, so the final source digest
    cannot save this test.  The watchdog callback must first fsync the finding,
    remain alive, and hand the refusal to the worker running the plan.
    """
    config, victim = _checkout(tmp_path)
    plan = Plan("release")

    with (
        pytest.raises(GateError, match="source tree changed during a release"),
        RunLog.open(config, "release") as log,
        observing(config, log, plan, publishes=True) as watch,
    ):
        errors = log.directory / config.runlog.error_log
        assert watch is not None
        runner = GuardedRunner(
            # This is deliberately a minimal synthetic Git checkout rather
            # than a second Capsem checkout.  Supply the authoritative policy
            # already loaded above instead of asking Runner to find another
            # config/gate.toml inside the fixture.
            Runner(config.root, stop_policy=STOP_POLICY),
            journal=log,
            checkpoint=watch.checkpoint,
        )
        plan.add(step("mutate", Run(_mutate_then_restore(victim, errors))))
        try:
            plan.run(Context(runner, config, journal=log, watch=watch))
        except GateError:
            assert watch._observer is not None and watch._observer.is_alive(), (
                "the refusal escaped on watchdog's thread and killed observation"
            )
            raise

    assert victim.read_text(encoding="utf-8") == "before\n", (
        "the final source digest would see no change; live observation is the proof"
    )
    assert "[source-tree]" in errors.read_text(encoding="utf-8")
    assert "HERMETICITY FATAL [source-tree]" in capfd.readouterr().err

    events = read(log.directory, config.runlog)
    fault_note = next(
        index
        for index, event in enumerate(events)
        if event.get("event") == "note"
        and str(event.get("message", "")).startswith("fault source-tree:")
    )
    command_end = next(index for index, event in enumerate(events) if event.get("event") == "exec")
    failed_step = next(
        index
        for index, event in enumerate(events)
        if event.get("event") == "step.end" and event.get("status") == "failed"
    )
    run_end = next(index for index, event in enumerate(events) if event.get("event") == "run.end")
    assert max(fault_note, command_end) < failed_step < run_end, (
        "the finding and command result must be durable before failure unwinds"
    )
    assert (
        json.loads((log.directory / config.runlog.events).read_text().splitlines()[-1])["status"]
        == "failed"
    )


def test_pending_fault_stops_before_the_next_in_process_action(tmp_path: Path) -> None:
    """File primitives do not cross a refusal just because no subprocess ran."""
    ran: list[str] = []
    watch = Watch([], source_root=tmp_path)

    class Arm(Action, name="arm-refusal"):
        def render(self) -> str:
            return "arm a source refusal"

        def perform(self, context: Context) -> None:
            ran.append("arm")
            watch.refuse("source changed asynchronously")

    class TooLate(Action, name="too-late"):
        def render(self) -> str:
            return "work after the refusal"

        def perform(self, context: Context) -> None:
            ran.append("too-late")

    plan = Plan("checkpoint")
    plan.add(step("work", Arm(), TooLate()))

    with pytest.raises(GateError, match="source changed asynchronously"):
        plan.run(Context(Runner(tmp_path), CONFIG, watch=watch))

    assert ran == ["arm"]


def test_launch_records_its_pid_before_the_post_action_refusal(tmp_path: Path) -> None:
    """A source refusal cannot strand a daemon before teardown can identify it."""
    watch = Watch([], source_root=tmp_path)

    class ArmsDuringLaunch(Runner):
        def launch(self, argv, *, env=None, cwd=None, secret_env=frozenset()) -> int:
            del argv, env, cwd, secret_env
            watch.refuse("source changed while the daemon started")
            return 12345

    journal = NullJournal()
    runner = GuardedRunner(ArmsDuringLaunch(tmp_path), journal=journal, checkpoint=watch.checkpoint)
    pidfile = tmp_path / "daemon.pid"
    plan = Plan("launch-checkpoint")
    plan.add(step("daemon", Launch(("daemon",), pidfile=pidfile)))

    with pytest.raises(GateError, match="source changed while the daemon started"):
        plan.run(Context(runner, CONFIG, journal=journal, watch=watch))

    assert pidfile.read_text(encoding="utf-8") == "12345"
