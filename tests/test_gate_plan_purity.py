"""A plan is built from source, not from whatever the last run left behind.

`module_functional` asked `profiles.selected(config)` for its axis while the
plan was being *constructed*, and that reads `target/config/profiles` -- build
output. So the same commit produced one plan on a warm tree and a different
one on a cold checkout.

That is not a theoretical hazard. `just release-profile nightly code` passed a
57-minute gate locally, pushed, dispatched, and CI failed with 94 tests all
reporting `no materialized profiles found under target/config/profiles`. The
local run had been green partly on leftovers, and `source.record` /
`source.verify` could not have caught it: they digest tracked source, and this
input is not tracked source.

No step ordering fixes it. Plan construction is deliberately pure -- see
`command.py::_describe`, which builds against a runner that refuses every
invocation -- so a step's output cannot exist by the time the plan is built.
The axis has to come from `config/profiles/`, which is checked in, present on
every clone, and covered by the source digest. Agreement between that and what
was materialized is a *step*, and it runs after the step that materializes.
"""

from __future__ import annotations

import argparse
import shutil
from pathlib import Path

import pytest
from capsem_builder.gate import cli  # noqa: F401 - imported so every command registers
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.command import GateCommand
from capsem_builder.gate.sourcecommit import SourceCommit

PROJECT_ROOT = Path(__file__).resolve().parents[1]


#: What each command needs beyond the common flags. Release lanes take a
#: channel; the profile lane also takes a profile.
ARGUMENTS: dict[str, dict[str, str]] = {
    "release-binaries": {"channel": "nightly", "source_commit": SourceCommit("0" * 40)},
    "release-profile": {
        "channel": "nightly",
        "profile": "code",
        "source_commit": SourceCommit("0" * 40),
    },
}


def _plan_labels(name: str) -> tuple[str, ...]:
    from helpers.gate import RecordingRunner

    command = GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **ARGUMENTS.get(name, {})),
    )
    return tuple(command._describe().labels)


def test_the_functional_plan_is_the_same_shape_on_a_cold_tree(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """The 94-failure bug, stated as an equality.

    Move the materialized profiles out of the way -- which is what a fresh
    clone and every CI runner look like -- and the plan must not change.
    """
    config = gate_config.load(PROJECT_ROOT)
    materialized = config.path(config.suites.pytest.materialized_profiles)

    warm = _plan_labels("test-functional")

    stash = tmp_path / "profiles"
    moved = materialized.exists()
    if moved:
        shutil.move(str(materialized), str(stash))
    try:
        cold = _plan_labels("test-functional")
    finally:
        if moved:
            materialized.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(stash), str(materialized))

    assert cold == warm, (
        "the plan changed shape because build output was missing; a fresh "
        "clone therefore runs a different gate than a warm tree, which is how "
        "94 tests passed locally and failed in CI on the same commit"
    )


@pytest.mark.parametrize(
    "name", ["test-functional", "test-candidate", "release-binaries", "release-profile"]
)
def test_a_plan_builds_without_any_build_output(
    name: str, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """Every command whose plan reaches the functional axis.

    Parametrized rather than looped so a regression names which command broke,
    not merely that one did.
    """
    config = gate_config.load(PROJECT_ROOT)
    materialized = config.path(config.suites.pytest.materialized_profiles)

    stash = tmp_path / f"profiles-{name}"
    moved = materialized.exists()
    if moved:
        shutil.move(str(materialized), str(stash))
    try:
        labels = _plan_labels(name)
    finally:
        if moved:
            materialized.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(stash), str(materialized))

    assert labels, f"{name} produced an empty plan"


def test_the_axis_agreement_is_a_step_and_runs_after_materialization() -> None:
    """The check does not disappear, it moves to where it can run.

    Materialized, declared and source axes still have to agree -- a materialized
    catalog that differs from the manifest means the gate would prove a pairing
    nobody is shipping. That is a run-time question, so it is a step, and it
    depends on the step that materializes.
    """
    from helpers.gate import gate_labels

    # In the functional module alone there is nothing to materialize, so the
    # claim is that the check runs before anything depends on it.
    alone = gate_labels("test-functional")
    assert "functional.axis" in alone, alone
    assert alone.index("functional.axis") == 0, alone[:3]

    # In the complete gate it must come after the step that materializes --
    # asserted as ordering rather than a direct edge, because the intervening
    # shape is the plan's business and pinning it would break on any reshuffle.
    whole = gate_labels("test-candidate")
    assert whole.index("prepare.materialize-config") < whole.index("functional.axis"), (
        "the axis is checked before anything materializes it"
    )


@pytest.mark.parametrize("name", ["release-binaries", "release-profile"])
def test_the_release_plan_is_byte_identical_without_build_output(name: str, tmp_path: Path) -> None:
    """Stronger than "it builds": the plan must be the *same* plan.

    A release lane that merely plans on a cold tree could still plan something
    different -- fewer profiles, a skipped lane -- and publish on the strength
    of a proof that never ran. Verified once by cloning to a directory with no
    `target/` and diffing the dry run (zero lines); asserted here so it stays
    true without a clone.
    """
    from helpers.gate import RecordingRunner

    config = gate_config.load(PROJECT_ROOT)
    materialized = config.path(config.suites.pytest.materialized_profiles)

    def described() -> str:
        command = GateCommand.registry[name](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False, **ARGUMENTS.get(name, {})),
        )
        return command._describe().describe()

    warm = described()
    stash = tmp_path / f"cold-{name}"
    moved = materialized.exists()
    if moved:
        shutil.move(str(materialized), str(stash))
    try:
        cold = described()
    finally:
        if moved:
            materialized.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(stash), str(materialized))

    assert cold == warm, f"{name} plans a different release without build output"
