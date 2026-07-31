"""The gate must count processes, not just call cleanup code.

`test_pidfile_cleanup_is_wired.py` proves every pidfile the gate stops is one
some binary writes. That is a claim about wiring. It stayed green through the
entire bug: the pidfile was real, the reaping call was real, and the reap did
nothing because a losing service starter had deleted the file. Six services and
their trays survived a session of release-lane runs, each reparented to launchd,
and every run reported success.

This file guards the other half -- that the gate ends by counting what is still
alive, and that a survivor fails the run.
"""

from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

import pytest

from helpers.sign import sign_binary


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check-orphan-processes.py"
JUSTFILE = ROOT / "justfile"

SPEC = importlib.util.spec_from_file_location("check_orphan_processes_guard", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
ORPHANS = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = ORPHANS
SPEC.loader.exec_module(ORPHANS)


def _trap_body(recipe: str) -> str:
    """The trap's executable lines, without its comments.

    These assertions are about what the shell runs. A comment that quotes the
    wrong-but-plausible form -- and this one deliberately does -- must not read
    as the code doing it.
    """
    body = recipe.split("close_out_candidate() {", maxsplit=1)[1].split(
        "\n    }", maxsplit=1
    )[0]
    return "\n".join(
        line for line in body.splitlines() if not line.lstrip().startswith("#")
    )


def _test_recipe() -> str:
    lines = JUSTFILE.read_text(encoding="utf-8").splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith("test:"))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        if lines[i] and not lines[i].startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


# ---------------------------------------------------------------------------
# Wiring: the count has to actually run, and has to be able to fail the gate
# ---------------------------------------------------------------------------


def test_gate_takes_a_baseline_and_checks_it() -> None:
    recipe = _test_recipe()

    assert "check-orphan-processes.py baseline" in recipe, (
        "without a baseline the check cannot tell this run's processes from a "
        "developer's own dev daemon, so it can only be reckless or useless"
    )
    assert "check-orphan-processes.py check" in recipe
    assert recipe.index("check-orphan-processes.py baseline") < recipe.index(
        "check-orphan-processes.py check"
    )


def test_orphan_check_runs_even_when_the_gate_aborts() -> None:
    """The check belongs in the EXIT trap, and the trap must stay armed.

    An aborted run is the one that skips its cleanup, so it is exactly the run
    whose processes need counting. A `trap - EXIT` before the end would disarm
    the count on the path that needs it most.
    """
    recipe = _test_recipe()
    trap_body = _trap_body(recipe)

    assert "check-orphan-processes.py check" in trap_body
    assert "trap close_out_candidate EXIT" in recipe
    assert "trap - EXIT" not in recipe, (
        "disarming the trap skips the process count on the abort path"
    )


def test_a_survivor_fails_the_recipe() -> None:
    """The trap raises the status on a leak, and never lowers it otherwise.

    `exit "$status"` looks equivalent to `return "$status"` and is not: `$?` at
    trap time is the last command's, so on an interrupted run it is 0, and
    exiting with it would discard the shell's own non-zero status and report
    the abort as a pass. Only the leak path may force a code.
    """
    recipe = _test_recipe()
    trap_body = _trap_body(recipe)

    assert "check-orphan-processes.py check || exit 1" in trap_body, (
        "a leak must fail the recipe; printing it and returning would leave "
        "the gate green, which is the whole failure being fixed here"
    )
    assert 'return "$status"' in trap_body
    assert 'exit "$status"' not in trap_body, (
        "exiting with the trap-time status turns an aborted run into a pass"
    )


# ---------------------------------------------------------------------------
# Classification: who gets blamed
# ---------------------------------------------------------------------------


def _facts(pid: int, created: float, exe: str = "/repo/target/debug/capsem-service") -> dict:
    return {
        "pid": pid,
        "name": Path(exe).name,
        "cmdline": exe,
        "exe": exe,
        "created": created,
        "ppid": 1,
    }


def test_processes_that_predate_the_run_are_not_blamed() -> None:
    current = {10: _facts(10, 100.0), 11: _facts(11, 200.0)}

    leaked = ORPHANS.started_during_run(current, {10: 100.0})

    assert set(leaked) == {11}


def test_a_recycled_pid_is_not_waved_through() -> None:
    """Same pid, different start time, across a multi-hour gate."""
    current = {10: _facts(10, 900.0)}

    leaked = ORPHANS.started_during_run(current, {10: 100.0})

    assert set(leaked) == {10}, (
        "a pid recycled onto a fresh capsem process would otherwise pass as "
        "pre-existing -- precisely the leak this exists to catch"
    )


def test_a_process_outside_the_checkout_is_never_ours(tmp_path: Path) -> None:
    installed = _facts(12, 100.0, exe=str(Path.home() / ".capsem/bin/capsem-service"))

    assert not ORPHANS._from_this_tree(installed, ROOT), (
        "the user's installed daemon is not the gate's to reap"
    )


def test_a_process_from_this_checkout_is_ours() -> None:
    ours = _facts(13, 100.0, exe=str(ROOT / "target/debug/capsem-service"))

    assert ORPHANS._from_this_tree(ours, ROOT)


# ---------------------------------------------------------------------------
# Functional: the check finds and reaps a real survivor
# ---------------------------------------------------------------------------


@pytest.mark.skipif(not Path("/bin/sleep").exists(), reason="needs /bin/sleep")
def test_check_detects_and_reaps_a_real_orphan(tmp_path: Path) -> None:
    """End-to-end against a live process, scoped to a throwaway root.

    Scoped deliberately: sweeping the real checkout here would let this test
    SIGKILL a sibling xdist worker's live service.
    """
    fake_root = tmp_path / "checkout"
    (fake_root / "target" / "debug").mkdir(parents=True)
    binary = fake_root / "target" / "debug" / "capsem-fake-leak"
    shutil.copy("/bin/sleep", binary)
    # Copying strips the signature, and Apple Silicon kills an unsigned binary
    # at exec. Same ad-hoc signing every other test fixture uses; a no-op on
    # Linux.
    sign_binary(binary)

    baseline_file = tmp_path / "baseline.json"
    assert ORPHANS.main(["baseline", "--baseline-file", str(baseline_file),
                         "--root", str(fake_root)]) == 0
    assert json.loads(baseline_file.read_text()) == {}

    leaked = subprocess.Popen([str(binary), "300"])
    try:
        deadline = time.time() + 10
        while time.time() < deadline:
            if ORPHANS.repo_capsem_processes(fake_root):
                break
            time.sleep(0.05)
        assert ORPHANS.repo_capsem_processes(fake_root), "fake orphan never became visible"

        status = ORPHANS.main(["check", "--baseline-file", str(baseline_file),
                               "--root", str(fake_root)])

        assert status == 1, "a process that outlived the gate must fail the gate"
        assert leaked.poll() is not None, "the orphan must be reaped, not just reported"
    finally:
        if leaked.poll() is None:
            leaked.kill()
        leaked.wait()


def test_baseline_announces_leftovers_from_an_earlier_run(tmp_path: Path, capsys) -> None:
    """A process the baseline absorbs is one the check can never flag again.

    A run killed outright never reaches its trap, so its orphans are still
    there when the next gate takes its baseline. Recording them silently makes
    them permanently invisible -- which is how they reach five hours old.
    """
    fake_root = tmp_path / "checkout"
    (fake_root / "target" / "debug").mkdir(parents=True)
    binary = fake_root / "target" / "debug" / "capsem-fake-leftover"
    shutil.copy("/bin/sleep", binary)
    sign_binary(binary)

    leftover = subprocess.Popen([str(binary), "300"])
    try:
        deadline = time.time() + 10
        while time.time() < deadline and not ORPHANS.repo_capsem_processes(fake_root):
            time.sleep(0.05)

        baseline_file = tmp_path / "baseline.json"
        assert ORPHANS.main(["baseline", "--baseline-file", str(baseline_file),
                             "--root", str(fake_root)]) == 0

        err = capsys.readouterr().err
        assert "already running before the gate started" in err
        assert str(leftover.pid) in err

        # Still not blamed on this run: the point is visibility, not a new
        # failure mode for a developer's own dev daemon.
        assert ORPHANS.main(["check", "--baseline-file", str(baseline_file),
                             "--root", str(fake_root)]) == 0
        assert leftover.poll() is None, "a pre-existing process must not be reaped"
    finally:
        leftover.kill()
        leftover.wait()


def test_check_refuses_to_guess_without_a_baseline(tmp_path: Path, capsys) -> None:
    status = ORPHANS.main(["check", "--baseline-file", str(tmp_path / "absent.json"),
                           "--root", str(tmp_path)])

    assert status == 2, "a missing baseline is a wiring bug, not a clean run"
    assert "no process baseline" in capsys.readouterr().err


def test_clean_run_passes(tmp_path: Path) -> None:
    baseline_file = tmp_path / "baseline.json"
    ORPHANS.main(["baseline", "--baseline-file", str(baseline_file), "--root", str(tmp_path)])

    assert ORPHANS.main(["check", "--baseline-file", str(baseline_file),
                         "--root", str(tmp_path)]) == 0
