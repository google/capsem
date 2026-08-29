"""`just doctor` reports on the machine, and this proves it in two halves.

This launched `just doctor` as a subprocess. That works from a shell and
cannot work from inside `just test-clean`, because the recipe depends on
`_pnpm-install`, which dispatches to `capsem-gate install-node`, which takes
the machine lock -- the lock the gate running this suite is holding. The child
would wait out its full timeout for a lock that cannot be released until its
own parent returns, which is the deadlock the composition model exists to make
unrepresentable, and `GateCommand` refuses it by name.

So the claim is split where the architecture splits it. That the recipe
dispatches to the right commands is read from the justfile; that those commands
report a healthy checkout is `capsem-gate doctor`, asserted in-process, which
is legal inside a held lock and is the half that can actually fail.
"""

import os
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]

pytestmark = pytest.mark.recipe


def _recipe(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8").splitlines()
    start = next(
        index
        for index, line in enumerate(lines)
        if line.startswith((f"{name}:", f"{name} "))
    )
    end = len(lines)
    for index in range(start + 1, len(lines)):
        if lines[index] and not lines[index].startswith((" ", "\t", "#")):
            end = index
            break
    return "\n".join(lines[start:end])


def test_just_doctor_dispatches_to_both_halves_of_the_check():
    """The gate checks its own wiring; the script checks the machine."""
    recipe = _recipe("doctor")

    assert "uv run --project build_system --frozen capsem-gate doctor" in recipe
    assert "build_system/scripts/doctor/doctor-common.sh" in recipe
    # Node workspaces first: the gate's own check reads the web surfaces.
    assert "_pnpm-install" in recipe.splitlines()[0]


def test_this_checkout_passes_the_doctor_it_dispatches_to():
    """The half that can fail, run for real -- in-process, so it never asks
    for a lock this suite's own gate is holding."""
    from capsem_builder.gate import doctor
    from helpers.gate import RecordingRunner

    assert doctor.check(RecordingRunner(PROJECT_ROOT)) == []


def test_launching_a_recipe_that_takes_the_machine_lock_is_refused(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Stated, because this file used to do exactly that.

    A suite running inside `just test-clean` cannot shell out to a recipe whose
    graph reaches an exclusive command. The refusal is what keeps a
    forty-minute run from becoming a two-hour timeout, so it is worth a test
    rather than a comment.
    """
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.command import GateCommand
    from capsem_builder.gate.errors import GateError
    from helpers.gate import RecordingRunner

    marker = gate_config.load(PROJECT_ROOT).locks.gate.run_marker
    monkeypatch.setenv(marker, "capsem-gate candidate")

    command = GateCommand.registry["install-node"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )

    with pytest.raises(GateError, match="machine lock"):
        command.execute()


def test_the_marker_is_what_a_running_gate_exports() -> None:
    """Otherwise the refusal above is theatre.

    `ExclusiveLock` exports it for the length of a run, which is what makes a
    child able to notice it is inside one.
    """
    from capsem_builder.gate import config as gate_config

    marker = gate_config.load(PROJECT_ROOT).locks.gate.run_marker
    locks = (PROJECT_ROOT / "build_system/builder/gate/locks.py").read_text(encoding="utf-8")

    assert "run_marker" in locks
    assert marker == "CAPSEM_GATE_RUN"
    assert os.environ.get(marker) is None or os.environ[marker]
