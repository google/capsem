"""What a command actually printed, kept with the step that ran it.

`RunLog.step_log()` existed and the module documentation promised a log per
step, and no production code called it. A real recorded `release-binaries` run
in this checkout had a `steps/` directory with zero files in it: the compiler
output, the pytest output, the Docker output and every script's output existed
only as terminal scrollback, for whoever happened to be watching.

That is the half of "a failed gate is a directory you can attach" that was
missing. The events said which command failed and with what status; nothing
said what it printed on the way.

Capture is a property of the funnel here, not of a call site remembering to
pass `log=`. The cost is real and deliberate: output goes through a pipe, so a
child no longer sees a TTY. Evidence beats progress bars.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.actions import Run
from capsem.gate.errors import GateError
from capsem.gate.execution import step
from capsem.gate.funnel import GuardedRunner
from capsem.gate.proc import Runner
from capsem.gate.runlog import RunLog

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SOURCE = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")


def _checkout(tmp_path: Path, **overrides: object) -> gate_config.GateConfig:
    (tmp_path / "config").mkdir(parents=True, exist_ok=True)
    source = SOURCE
    for key, value in overrides.items():
        original = next(line for line in source.splitlines() if line.startswith(f"{key} = "))
        source = source.replace(original, f"{key} = {value}")
    (tmp_path / "config" / "gate.toml").write_text(source, encoding="utf-8")
    return gate_config.load(tmp_path)


#: Writes to both streams, so "combined" is a claim a test can check.
BOTH = (
    "import sys; "
    "print('OUT-MARKER'); "
    "print('ERR-MARKER', file=sys.stderr); "
    "sys.stderr.flush()"
)


def _run(config, label: str, script: str, *, check: bool = True) -> Path:
    """One step, run for real, returning its log path."""
    import sys

    with RunLog.open(config, "test") as log:
        runner = GuardedRunner(Runner(config.root), journal=log)
        with log.step(step(label, Run([sys.executable, "-c", script]))):
            runner.run([sys.executable, "-c", script], check=check)
        return log.step_log(label)


def test_a_step_keeps_what_its_commands_printed(tmp_path: Path) -> None:
    config = _checkout(tmp_path)

    written = _run(config, "build", BOTH)

    assert written.is_file(), "the step wrote no log at all"
    body = written.read_text(encoding="utf-8")
    assert "OUT-MARKER" in body
    assert "ERR-MARKER" in body, "stderr is where compilers put the part you need"


def test_the_operator_still_sees_it_happen(
    tmp_path: Path, capfd: pytest.CaptureFixture[str]
) -> None:
    """Captured, not swallowed. A forty-minute gate that prints nothing until
    it finishes is a gate nobody can tell from a hung one."""
    config = _checkout(tmp_path)

    _run(config, "build", BOTH)

    streamed = capfd.readouterr()
    assert "OUT-MARKER" in streamed.out + streamed.err


def test_concurrent_steps_never_share_a_file(tmp_path: Path) -> None:
    """Two lanes interleaved into one terminal is unreadable, and one file
    attributed to the wrong step is worse than no file."""
    import sys
    import threading

    config = _checkout(tmp_path)
    with RunLog.open(config, "test") as log:
        runner = GuardedRunner(Runner(config.root), journal=log)

        def lane(name: str) -> None:
            with log.step(step(name, Run(["true"]))):
                runner.run([sys.executable, "-c", f"print('{name}-MARKER')"])

        threads = [threading.Thread(target=lane, args=(name,)) for name in ("alpha", "beta")]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()

        alpha = log.step_log("alpha").read_text(encoding="utf-8")
        beta = log.step_log("beta").read_text(encoding="utf-8")

    assert "alpha-MARKER" in alpha and "beta-MARKER" not in alpha
    assert "beta-MARKER" in beta and "alpha-MARKER" not in beta


def test_a_failure_carries_its_own_tail(tmp_path: Path) -> None:
    """Without terminal capture. The status alone does not say what broke."""
    config = _checkout(tmp_path, failure_tail_lines=3)
    script = (
        "import sys; "
        "[print(f'line-{n}') for n in range(50)]; "
        "print('THE-REAL-ERROR', file=sys.stderr); "
        "sys.exit(3)"
    )

    with pytest.raises(GateError) as raised:
        _run(config, "build", script)

    message = str(raised.value)
    assert "THE-REAL-ERROR" in message
    assert "line-0" not in message, "the whole log belongs in steps/, not the error"
    assert len(re.findall(r"line-\d+", message)) <= 3


def test_a_command_outside_any_step_still_runs(tmp_path: Path) -> None:
    """Resources acquire and release outside the step graph.

    There is no step to attribute their output to, and refusing to run them
    would be a worse answer than not filing the log.
    """
    import sys

    config = _checkout(tmp_path)
    with RunLog.open(config, "test") as log:
        runner = GuardedRunner(Runner(config.root), journal=log)
        assert runner.run([sys.executable, "-c", "print('loose')"]) == 0


def test_captured_output_is_still_returned_to_its_caller(tmp_path: Path) -> None:
    """`capture` output is data a caller parses, not narration.

    Teeing it would be harmless; losing it would silently change what every
    probe in the gate decides.
    """
    import sys

    config = _checkout(tmp_path)
    with RunLog.open(config, "test") as log:
        runner = GuardedRunner(Runner(config.root), journal=log)
        with log.step(step("probe", Run(["true"]))):
            answer = runner.capture([sys.executable, "-c", "print('the-answer')"])

    assert answer == "the-answer"
