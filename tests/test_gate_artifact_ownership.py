"""The artifact contract has real producers, so its guard has real data.

`Step.produces` drove two things -- hashing what a step produced into the run
log, and the "one owner per artifact" check -- and no production step supplied
it. So `Hash` had no caller through that mechanism, every run log recorded zero
artifacts, and the ownership check ran over an empty set and could never fire.

That is worse than not having the abstraction: the guard existed, was green,
and answered a question nobody had asked it.

Declared at the fragment that builds the bytes rather than at each composer, so
every command containing that fragment inherits the claim.
"""

from __future__ import annotations

from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: Producers the complete gate must declare. Each is a path a later step, a
#: release lane, or an operator reads -- which is what makes "who wrote this"
#: a question worth being able to answer.
EXPECTED = {
    "prepare.sign",
    "static.guest-agents",
    "initrd.repack",
    "source.record",
}


def _plan(command: str = "candidate"):
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_plan

    return gate_plan(command)


def _producers(plan) -> dict[str, tuple[Path, ...]]:
    return {step.label: step.produces for step in plan.steps if step.produces}


def test_the_complete_gate_declares_who_produces_its_artifacts() -> None:
    declared = _producers(_plan())

    assert set(declared) >= EXPECTED, (
        f"these steps write bytes something later reads and declare nothing: "
        f"{sorted(EXPECTED - set(declared))}"
    )


def test_every_declared_artifact_is_owned_or_serialized() -> None:
    """The check that finally has data to run over.

    Not "exactly one producer": the host binaries are signed at three points
    in a composed gate, deliberately, and that is safe *because* those steps
    contend for the same exclusive. The rule is the one `planchecks` enforces
    -- two writers must share something that serializes them, or a consumer
    ordered after its producer can still read what another one overwrote.
    """
    plan = _plan()
    owners: dict[Path, list] = {}
    for step in plan.steps:
        for path in step.produces:
            owners.setdefault(path, []).append(step)

    unguarded = {}
    for path, producers in owners.items():
        if len(producers) < 2:
            continue
        # Exclusive claims only. This repeated the validator's name-only
        # intersection, so the test agreed with the bug it was meant to catch:
        # two producers both holding one name *shared* overlap by design.
        serializing = set.intersection(
            *(
                {resource.name for resource in step.contends if not resource.shared}
                for step in producers
            )
        )
        if not serializing:
            unguarded[path] = sorted(step.label for step in producers)

    assert not unguarded, f"these paths have racing producers: {unguarded}"


def test_the_ownership_check_is_no_longer_running_over_nothing() -> None:
    """The guard existed, was green, and had been asked no question at all."""
    plan = _plan()
    declared = [path for step in plan.steps for path in step.produces]

    assert len(declared) > 5, (
        "with no production step declaring `produces`, `require_one_owner_per_"
        "artifact` iterates an empty set and can never fire"
    )


@pytest.mark.parametrize("command", ["candidate", "release-binaries", "release-profile"])
def test_both_release_lanes_inherit_the_same_producers(command: str) -> None:
    """Declared at the fragment, so composing it carries the claim along."""
    args = {
        "release-binaries": {"channel": "nightly"},
        "release-profile": {"channel": "nightly", "profile": "code"},
    }.get(command, {})
    import argparse
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    plan = GateCommand.registry[command](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )._describe()

    assert set(_producers(plan)) >= EXPECTED


# ---------------------------------------------------------------------------
# What "serialized" has to mean, now that a claim can be shared
# ---------------------------------------------------------------------------


def _two_producers(shared: tuple[bool, bool]):
    """Two steps writing one path, each claiming `docker_daemon`."""
    from capsem.gate import config as gate_config
    from capsem.gate.execution import step
    from capsem.gate.plan import Plan

    config = gate_config.load(PROJECT_ROOT)
    artifact = PROJECT_ROOT / "target" / "contested.bin"
    plan = Plan("synthetic")
    for index, is_shared in enumerate(shared):
        claim = (
            config.shared("docker_daemon") if is_shared else config.exclusive("docker_daemon")
        )
        plan.add(step(f"writer-{index}", produces=(artifact,), contends=(claim,)))
    return plan, config


def test_two_shared_claims_do_not_serialize_their_writers() -> None:
    """A shared claim is a readers-lock: both hold it at once, by design.

    The guard intersected contention *names* and stopped there, so two lanes
    both holding `docker_daemon` shared passed validation and were free to
    overwrite one path concurrently. The name was common; the exclusion was
    never there.
    """
    from capsem.gate.errors import GateError

    plan, config = _two_producers((True, True))

    with pytest.raises(GateError, match="shared"):
        plan.validate(config)


def test_one_shared_and_one_exclusive_claim_still_overlap() -> None:
    """The shared holder is not excluded by the other's exclusivity alone --
    it is the *writer* that waits, and two writers is the case here."""
    from capsem.gate.errors import GateError

    plan, config = _two_producers((True, False))

    with pytest.raises(GateError):
        plan.validate(config)


def test_two_exclusive_claims_on_one_name_are_accepted() -> None:
    """Which is the arrangement the host binaries actually use."""
    plan, config = _two_producers((False, False))

    plan.validate(config)
