"""A plan is built from source, not from whatever the last run left behind.

`module_functional` once asked for its profile axis while the plan was being
*constructed*, and that read `cache/target/config/profiles` -- build output. So
the same commit produced one plan on a warm tree and a different one on a cold
checkout.

That is not a theoretical hazard. A release passed a 57-minute gate locally,
pushed, dispatched, and CI failed with 94 tests all reporting `no materialized
profiles found under cache/target/config/profiles`. The local run had been
green partly on leftovers, and `source.record` / `source.verify` could not have
caught it: they digest tracked source, and this input is not tracked source.

The axis is gone (#289: one runtime), but the rule it taught stays. Plan
construction is deliberately pure -- see `command.py::_describe`, which builds
against a runner that refuses every invocation -- so a step's output cannot
exist by the time the plan is built. Whether the content a phase boots is
complete is a *step*, and it runs after the step that produces the content.
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

PROJECT_ROOT = Path(__file__).resolve().parents[3]


#: What each command needs beyond the common flags. Release lanes take a
#: channel and the exact source commit.
ARGUMENTS: dict[str, dict[str, str]] = {
    "release-binaries": {"channel": "nightly", "source_commit": SourceCommit("0" * 40)},
    "release-assets": {"channel": "nightly", "source_commit": SourceCommit("0" * 40)},
}


def _materialized(config) -> Path:
    """The generated catalog a warm tree has and a fresh clone does not."""
    return config.path(config.functional.config_root) / config.functional.profiles_subdir


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

    Move the materialized catalog out of the way -- which is what a fresh
    clone and every CI runner look like -- and the plan must not change.
    """
    config = gate_config.load(PROJECT_ROOT)
    materialized = _materialized(config)

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
    "name", ["test-functional", "test-candidate", "release-binaries", "release-assets"]
)
def test_a_plan_builds_without_any_build_output(
    name: str, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """Every command whose plan reaches the functional suites.

    Parametrized rather than looped so a regression names which command broke,
    not merely that one did.
    """
    config = gate_config.load(PROJECT_ROOT)
    materialized = _materialized(config)

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


def test_the_content_check_is_a_step_and_runs_after_the_content_is_built() -> None:
    """The check does not disappear, it moves to where it can run.

    The complete gate boots the runtime IronBank built, so that the pair it
    points every suite at is complete is a run-time question: a step, after
    the assembly that produces it and before the first suite that boots it.
    """
    from helpers.gate import gate_labels

    whole = gate_labels("test-candidate")
    assert "functional.content" in whole, whole
    assert whole.index("assets.assemble") < whole.index("functional.content"), (
        "the content is checked before anything built it"
    )
    assert whole.index("functional.content") < whole.index("functional.pytest.broad")

    # The standalone owner boots the checkout's own runtime, so it must
    # materialize that first; otherwise it only passes on a warm checkout.
    alone = gate_labels("test-functional")
    assert alone.index("prepare.materialize-config") < alone.index("functional.pytest.broad"), (
        alone
    )


@pytest.mark.parametrize("name", ["release-binaries", "release-assets"])
def test_the_release_plan_is_byte_identical_without_build_output(name: str, tmp_path: Path) -> None:
    """Stronger than "it builds": the plan must be the *same* plan.

    A release lane that merely plans on a cold tree could still plan something
    different -- a skipped lane, a missing suite -- and publish on the strength
    of a proof that never ran. Verified once by cloning to a directory with no
    `cache/target/` and diffing the dry run (zero lines); asserted here so it stays
    true without a clone.
    """
    from helpers.gate import RecordingRunner

    config = gate_config.load(PROJECT_ROOT)
    materialized = _materialized(config)

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
