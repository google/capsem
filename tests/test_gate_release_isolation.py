"""A release consumes qualification and dispatches from one detached prefix.

Phase 7 gave `candidate` a private tree and stopped there, so both release
commands still spent the hour-long gate against the checkout a developer was
editing. That is the failure class the copy exists to remove, and a release is
where it costs the most.

The selected commit is already on main and is materialized as an independent
detached repository. A complete candidate journal is the only qualification
authority. Release revalidates that immutable journal at its first graph edge,
then publishes and dispatches from the same exact source. It must not repeat
the multi-hour proof or accept an operator-authored receipt in its place.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from helpers.gate import built_command

from capsem.gate import config as gate_config
from capsem.gate.candidate import CompleteGate
from capsem.gate.command import GateCommand

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)

RELEASES = [
    ("release-binaries", {"channel": "stable"}),
    ("release-profile", {"channel": "stable", "profile": "code"}),
]

#: The steps that reach outside this machine, or decide whether to. Everything
#: else in a release plan is the gate.
PUBLICATION = ("source.remote-main", "source.publish-ref", "release")

#: The one step whose subject *is* the tree being edited. Every other step must
#: read the detached copy, because measuring one tree while qualifying another
#: is the confusion this file exists to prevent. This step exists to report
#: that difference rather than be misled by it: a release publishes the commit,
#: so an uncommitted change is silently excluded unless something says so.
INSPECTS_THE_EDITED_TREE = ("source.worktree-clean",)

# The only steps in either local release command allowed to execute outside
# the kernel network boundary. The binary precheck reads the remote version
# tag; manifest resolution, source validation/publication and dispatch also
# genuinely need the network.
NETWORKED = {
    "release-binaries": (
        "channel-source",
        "precheck",
        "source.remote-main",
        "source.publish-ref",
        "release",
    ),
    "release-profile": (
        "source.remote-main",
        "source.publish-ref",
        "release",
    ),
}


def _plan(name: str, **args):
    return built_command(PROJECT_ROOT, name, tuple(args.items()))._describe()


@pytest.fixture
def checkout(tmp_path, monkeypatch) -> Path:
    """Stand where the originating checkout stands, under a nameable path.

    The plan reads it from the environment exactly as a prefixed child does,
    so what these tests build is the composition a real release builds.
    """
    source = tmp_path / "originating-checkout"
    source.mkdir()
    monkeypatch.setenv(CONFIG.environment.source_checkout, str(source))
    return source


def test_complete_candidate_and_release_dispatch_use_private_copies() -> None:
    """Qualification and publication both avoid the mutable outer checkout."""
    assert CompleteGate.private_checkout is True
    assert GateCommand.registry["candidate"].private_checkout is True

    for name, args in RELEASES:
        _plan(name, **args)  # register the command through the real CLI helper
        assert GateCommand.registry[name].private_checkout is True, (
            f"{name} would dispatch from the mutable outer checkout"
        )


@pytest.mark.parametrize(("name", "args"), RELEASES)
def test_publication_never_reaches_the_mutable_outer_checkout(name, args, checkout) -> None:
    """All source decisions use the independent detached prefix repository."""
    plan = _plan(name, **args)
    steps = {step.label: step for step in plan.steps}

    for label in PUBLICATION:
        rendered = "\n".join(steps[label].render())
        assert checkout.name not in rendered
        assert PROJECT_ROOT.name in rendered


@pytest.mark.parametrize(("name", "args"), RELEASES)
def test_qualification_acceptance_stays_in_the_copy(name, args, checkout) -> None:
    """Journal acceptance and dispatch both resolve from the detached source."""
    plan = _plan(name, **args)

    escaped = [
        step.label
        for step in plan.steps
        if step.label not in PUBLICATION + INSPECTS_THE_EDITED_TREE
        and checkout.name in "\n".join(step.render())
    ]

    assert not escaped, (
        "these steps qualify the release and must read the private copy, not "
        f"the tree being edited: {', '.join(escaped)}"
    )


@pytest.mark.parametrize(("name", "args"), RELEASES)
def test_release_dispatches_qualification_without_rerunning_a_local_gate(
    name, args, checkout
) -> None:
    """The release plan freezes source and invokes no nested local gate."""
    plan = _plan(name, **args)
    # Whole words: `--bin capsem-gateway` is a binary this gate builds, and a
    # substring match reads it as a nested gate invocation.
    words = {word for step in plan.steps for line in step.render() for word in line.split()}

    for launcher in ("capsem-gate", "just"):
        assert launcher not in words, (
            f"a release step launches {launcher!r}; qualification belongs to "
            "the hosted lane, not another local gate process"
        )

    # The clean-tree refusal is first: the hosted lane must receive exactly the
    # committed source the operator named.
    assert plan.labels[0] == "source.worktree-clean"
    assert "qualification.accept" not in plan.labels
    for phase in ("fast.", "static.", "artifacts.", "functional.", "glowup."):
        assert not any(step.label.startswith(phase) for step in plan.steps), (
            f"the release runs local {phase} work owned by the hosted lane"
        )


def test_no_operator_authored_release_receipt_was_invented() -> None:
    """The runner-owned event journal is evidence; config has no second receipt."""
    settings = CONFIG.release.model_dump()

    assert "receipt" not in settings, (
        "an operator-authored release receipt would bypass the runner-owned "
        "content-addressed qualification journal"
    )


@pytest.mark.parametrize(("name", "args"), RELEASES)
def test_only_networked_release_edges_cross_the_kernel_boundary(name, args) -> None:
    """Qualification stays sandboxed while the irreducible network edges do not.

    The marker is part of the action's dry-run rendering, so this assertion is
    over the real composed plan rather than a second list maintained beside it.
    """
    plan = _plan(name, **args)
    marked = {
        step.label
        for step in plan.steps
        if any("[outside kernel sandbox]" in line for line in step.render())
    }

    assert marked == set(NETWORKED[name])
    assert not any(
        label.startswith(("artifacts.", "functional.", "glowup."))
        or (label.startswith(("fast.", "static.")) and label not in NETWORKED[name])
        for label in marked
    )


def test_force_does_not_invent_a_local_qualification_waiver() -> None:
    """`--force` adds source guards; hosted qualification remains mandatory."""
    from helpers.gate import built_command

    commit = "f" * 40
    forced = built_command(
        PROJECT_ROOT,
        "release-binaries",
        (("channel", "stable"), ("source_commit", commit), ("force", "true")),
        None,
    )._describe()
    assert "qualification.accept" not in forced.labels
    assert "qualification.waived" not in forced.labels
    assert "source.worktree-clean" not in forced.labels


def test_a_forced_release_still_proves_its_source() -> None:
    """`--force` waives the artifacts, not every check.

    The qualification proves the product, and the commits worth forcing are the
    ones that do not change it -- a workflow file, a gate policy. But force used
    to waive *everything*, so a forced release could dispatch source failing a
    six-second guard, and did: three dispatches in one afternoon died on a
    line-count ratchet and a pair of stale contracts, each found forty minutes
    into a lane instead of four minutes before it.

    The fit is exact. What gets forced are gate and CI changes, and citadel and
    the release contracts are the suites that judge exactly those.
    """
    from helpers.gate import built_command

    commit = "f" * 40
    forced = built_command(
        PROJECT_ROOT,
        "release-binaries",
        (("channel", "stable"), ("source_commit", commit), ("force", "true")),
        None,
    )._describe()

    ordered = list(forced.labels)
    for proof in ("citadel", "contracts.release"):
        assert proof in ordered, f"a forced release skips {proof}"
        assert ordered.index(proof) < ordered.index("release"), (
            f"{proof} must run before anything is published"
        )


def test_an_unforced_release_runs_no_local_qualification_suite() -> None:
    """A real release leaves qualification to the hosted lane.

    Adding the forced proof must not turn every release into a second gate --
    that is the reduced-versus-doubled gate the release contract refuses in
    both directions.
    """
    from helpers.gate import built_command

    ordinary = built_command(
        PROJECT_ROOT,
        "release-binaries",
        (("channel", "stable"), ("source_commit", "f" * 40)),
        None,
    )._describe()

    assert "citadel" not in ordinary.labels
    assert "contracts.release" not in ordinary.labels
    assert "qualification.accept" not in ordinary.labels
