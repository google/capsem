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
from capsem_builder.gate import host

PROJECT_ROOT = Path(__file__).resolve().parents[3]

#: Producers the complete gate must declare. Each is a path a later step, a
#: release lane, or an operator reads -- which is what makes "who wrote this"
#: a question worth being able to answer.
EXPECTED = {
    "prepare.sign" if host.on_macos() else "prepare.build-binaries",
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

    # Through the predicate the validator and the scheduler both use. This
    # reimplemented the rule, so the test agreed with the bug it was meant to
    # catch: two producers holding one name *shared* overlap by design, and a
    # name-only intersection called that serialized on both sides.
    from capsem_builder.gate.contention import can_overlap

    unguarded = {}
    for path, producers in owners.items():
        racing = [
            (first, second)
            for index, first in enumerate(producers)
            for second in producers[index + 1 :]
            if can_overlap(first, second)
        ]
        if racing:
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


@pytest.mark.parametrize("command", ["release-binaries", "release-profile"])
def test_release_lanes_consume_evidence_without_rebuilding_producers(command: str) -> None:
    """Publication accepts the complete run instead of rerunning its graph."""
    from capsem_builder.gate.sourcecommit import SourceCommit

    source_commit = SourceCommit("0" * 40)
    args = {
        "release-binaries": {"channel": "stable", "source_commit": source_commit},
        "release-profile": {
            "channel": "stable",
            "profile": "code",
            "source_commit": source_commit,
        },
    }.get(command, {})
    import argparse
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate.command import GateCommand
    from helpers.gate import RecordingRunner

    plan = GateCommand.registry[command](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )._describe()

    assert not _producers(plan)
    assert "qualification.accept" not in plan.labels
    assert "release" in plan.labels
    assert "source.record" not in plan.labels


# ---------------------------------------------------------------------------
# What "serialized" has to mean, now that a claim can be shared
# ---------------------------------------------------------------------------


def _two_producers(shared: tuple[bool, bool]):
    """Two steps writing one path, each claiming `docker_daemon`."""
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.execution import step
    from capsem_builder.gate.plan import Plan

    config = gate_config.load(PROJECT_ROOT)
    artifact = PROJECT_ROOT / "cache" / "target" / "contested.bin"
    plan = Plan("synthetic")
    for index, is_shared in enumerate(shared):
        claim = config.shared("docker_daemon") if is_shared else config.exclusive("docker_daemon")
        plan.add(step(f"writer-{index}", produces=(artifact,), contends=(claim,)))
    return plan, config


def test_two_shared_claims_do_not_serialize_their_writers() -> None:
    """A shared claim is a readers-lock: both hold it at once, by design.

    The guard intersected contention *names* and stopped there, so two lanes
    both holding `docker_daemon` shared passed validation and were free to
    overwrite one path concurrently. The name was common; the exclusion was
    never there.
    """
    from capsem_builder.gate.errors import GateError

    plan, config = _two_producers((True, True))

    with pytest.raises(GateError, match="shared"):
        plan.validate(config)


def test_one_shared_and_one_exclusive_claim_do_serialize() -> None:
    """Accepted, and deliberately.

    An exclusive holder admits nobody, so a shared claimant of the same
    resource cannot be in flight beside it -- and the scheduler enforces
    exactly that, because it reserves claims from the same predicate this
    validator uses. Rejecting the pair would be rejecting an arrangement that
    provably cannot race.
    """
    plan, config = _two_producers((True, False))

    plan.validate(config)


def test_two_exclusive_claims_on_one_name_are_accepted() -> None:
    """Which is the arrangement the host binaries actually use."""
    plan, config = _two_producers((False, False))

    plan.validate(config)
