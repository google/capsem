"""A release qualifies a private copy and publishes from the real checkout.

Phase 7 gave `candidate` a private tree and stopped there, so both release
commands still spent the hour-long gate against the checkout a developer was
editing. That is the failure class the copy exists to remove, and a release is
where it costs the most.

The first design for closing it split the release into three sibling processes
bound by a receipt. That is refused here, and the refusal is the point of this
file: `AGENTS.md` requires one process, one machine lock, one workspace and one
plan, requires each release command to *contain* the complete proof rather than
launch it, and forbids a parallel release ledger or result file. A receipt
binding three processes is that ledger.

So the isolation is composed inside the existing contract instead. One process
still, one plan still -- and inside it two territories:

  the *gate* runs from the copy, because its subject must not move while it is
  being measured

  *publication* runs in the checkout the copy was made from, because a commit,
  tag or push made in the copy lands in a `.git` that is reclaimed minutes
  later, and because the tree a human still has afterwards is the one being
  released

`require-source-unchanged` is what makes the pair sound: it compares the
originating checkout's HEAD *and* source digest against what was recorded, so
the two territories are provably the same bytes or the release stops.
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
    ("release-binaries", {"channel": "nightly"}),
    ("release-profile", {"channel": "nightly", "profile": "code"}),
]

#: The steps that reach outside this machine, or decide whether to. Everything
#: else in a release plan is the gate.
PUBLICATION = ("precheck", "record-head", "confirm-head", "release")

# The only steps in either local release command allowed to execute outside
# the kernel network boundary. Prechecks and the source-head capture are local
# filesystem/git reads; resolution, source publication and dispatch genuinely
# need the network.
NETWORKED = {
    "release-binaries": (
        "channel-source",
        "fast.audit.cargo",
        "fast.audit.pnpm",
        "fast.audit.python-lock",
        "confirm-head",
        "release",
    ),
    "release-profile": (
        "fast.audit.cargo",
        "fast.audit.pnpm",
        "fast.audit.python-lock",
        "confirm-head",
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


def test_a_command_containing_the_whole_gate_runs_it_from_a_copy() -> None:
    """Declared on the mixin, not on `candidate`.

    `CompleteGate` is precisely the set of commands that spend the complete
    forty-minute proof, which is precisely the set long enough for someone to
    edit the tree underneath. Declaring it per command left the two that
    publish -- the ones whose mistakes are public and irreversible -- as the
    only ones without it.
    """
    assert CompleteGate.private_checkout is True

    for name in ("candidate", *(name for name, _ in RELEASES)):
        assert GateCommand.registry[name].private_checkout is True, (
            f"{name} contains the complete gate and would run it against the live checkout"
        )


@pytest.mark.parametrize(("name", "args"), RELEASES)
def test_publication_reaches_the_checkout_the_copy_was_made_from(name, args, checkout) -> None:
    """A push from the copy pushes from a `.git` nobody keeps.

    The prefix carries a *copy* of `.git`, so a version stamp, a commit and a
    tag made there exist only until the prefix is reclaimed -- a release that
    reported success and published nothing a human could see.
    """
    plan = _plan(name, **args)
    steps = {step.label: step for step in plan.steps}

    for label in PUBLICATION:
        rendered = "\n".join(steps[label].render())
        assert checkout.name in rendered, (
            f"{label} publishes, so it must run in the originating checkout "
            f"rather than the copy:\n{rendered}"
        )


@pytest.mark.parametrize(("name", "args"), RELEASES)
def test_the_gate_itself_stays_in_the_copy(name, args, checkout) -> None:
    """The other half of the same claim, and the one that is easy to lose.

    Aiming a step at the checkout is one keyword, so the risk is not that
    publication misses it -- it is that the gate quietly acquires it and the
    isolation becomes decorative while every test above still passes.
    """
    plan = _plan(name, **args)

    escaped = [
        step.label
        for step in plan.steps
        if step.label not in PUBLICATION and checkout.name in "\n".join(step.render())
    ]

    assert not escaped, (
        "these steps qualify the release and must read the private copy, not "
        f"the tree being edited: {', '.join(escaped)}"
    )


@pytest.mark.parametrize(("name", "args"), RELEASES)
def test_the_release_is_still_one_plan_in_one_process(name, args, checkout) -> None:
    """`AGENTS.md` is the authority here, not this file's convenience.

    The rejected design made the gate a separate `capsem-gate` process between
    a fetch process and a publish process. Composed instead, "nothing publishes
    before the complete proof passes" stays an edge in one graph -- and the
    machine lock, which is not reentrant, is still taken exactly once.
    """
    plan = _plan(name, **args)
    # Whole words: `--bin capsem-gateway` is a binary this gate builds, and a
    # substring match reads it as a nested gate invocation.
    words = {word for step in plan.steps for line in step.render() for word in line.split()}

    for launcher in ("capsem-gate", "just"):
        assert launcher not in words, (
            f"a release step launches {launcher!r}; the gate is composed into "
            "this plan, and a second process would wait out its own parent's "
            "machine lock"
        )

    # The complete proof, present as phases rather than as a step named for it.
    for phase in ("fast.", "static.", "artifacts.", "functional.", "glowup."):
        assert any(step.label.startswith(phase) for step in plan.steps), (
            f"the {phase} phase is missing, so this plan is not the complete gate"
        )


def test_no_receipt_authority_was_invented() -> None:
    """The parallel ledger `AGENTS.md` forbids, refused by name.

    Splitting the release across processes needs something to bind them, and a
    receipt recording the fetched manifest, the HEAD, the source digest and the
    gate's result is exactly the "release result file" the release contract
    rules out. `record-head`/`confirm-head` already carry that guarantee inside
    one plan, so there is nothing for a second authority to add.
    """
    settings = CONFIG.release.model_dump()

    assert "receipt" not in settings, (
        "a release receipt is a parallel release ledger; the manifest is the "
        "bible and confirm-head is the fail-stop"
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
        label.startswith(("static.", "artifacts.", "functional.", "glowup."))
        or (label.startswith("fast.") and label not in NETWORKED[name])
        for label in marked
    )
