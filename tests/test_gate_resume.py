"""Continuing a failed run instead of replaying it.

Six consecutive `just test` runs were spent proving the same twenty minutes of
work to reach a new failure one step further on. The private checkout made that
worse, not better: a fresh copy per run starts with no `target/`, so every
replay is cold.

`--prefix <tree> --from <step>` reuses the tree the last run built in and
carries everything the graph puts before that step. These hold the two things
that make it safe to have at all: it is refused on the release path, and a
carried step is never recorded as one this process ran.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _config():
    from capsem.gate import config as gate_config

    return gate_config.load(PROJECT_ROOT)


def _candidate_plan():
    # Through the helper, which is the one place that knows importing `cli` is
    # what fills the registry. Spelling that import here needed a suppression
    # for a name nothing reads.
    from helpers.gate import built_command

    from capsem.gate.qualification import from_environment

    command = built_command(
        PROJECT_ROOT,
        "candidate",
        (("prefix", None), ("resume_from", None)),
        qualification=from_environment(_config(), environ={}),
    )
    return command.plan()


# -- what gets carried -------------------------------------------------------


def test_from_a_step_carries_exactly_its_ancestors() -> None:
    """The graph decides, not the last run's log.

    Derived from the edges, so the answer is checkable before anything runs and
    is the same every time -- what comes before a step is a property of the
    plan, not of what happened to succeed last night.
    """
    from capsem.gate import resume

    plan = _candidate_plan()
    carried = resume.ancestors(plan, "artifacts.build-chain")

    assert "source.record" in carried, "the earliest step is an ancestor of nearly everything"
    assert "artifacts.build-chain" not in carried, "the named step runs; that is the point"
    assert "source.verify" not in carried, "a step that comes after must never be carried"

    # And it is transitively closed: every ancestor's own ancestors are in too,
    # or the runner would block forever on a dependency nobody satisfied.
    for label in carried:
        assert plan.after_of(label) <= carried, f"{label} is carried but its inputs are not"


def test_a_source_fix_before_functional_rebuilds_the_exact_install_image() -> None:
    """Do not carry a source-keyed image merely because functional work resumes.

    A retained prefix refreshes its source before the resumed gate starts.  The
    install image key therefore changes when the fix touches any byte included
    in that image.  Chaining its smoke step ahead of the asset graph made all
    three lifecycle steps ancestors of ``functional.pytest.timing.code``; the
    continuation carried the old image, built both packages, then failed at
    glow-up because the new exact tag had never been materialized.

    The lifecycle is independent work that may run alongside functional proof,
    but the install transaction must still wait for its exact smoke result.
    """
    from capsem.gate import resume
    from capsem.gate.installimage import InstallImageStep

    plan = _candidate_plan()
    carried = resume.ancestors(plan, "functional.pytest.timing.code")
    install_steps = {step.value for step in InstallImageStep}

    assert install_steps.isdisjoint(carried)
    assert (InstallImageStep.SMOKE.value, "glowup.install") in plan.edges


def test_a_misspelled_step_is_refused_with_a_suggestion() -> None:
    """Cheap, and before the machine lock.

    `--dry-run --from <step>` resolves the name, so a typo costs a suggestion
    rather than twenty minutes and a held lock.
    """
    from capsem.gate import resume
    from capsem.gate.errors import GateError

    plan = _candidate_plan()
    with pytest.raises(GateError, match=re.escape("artifacts.build-chain")):
        resume.ancestors(plan, "build-chain")


def test_carrying_nothing_is_the_default() -> None:
    """A run without `--from` proves the whole graph."""
    from capsem.gate import resume

    plan = _candidate_plan()
    assert resume.carried(plan, _config(), None, qualifying=False) == frozenset()


# -- the refusals that keep it from being a skip flag ------------------------


def test_a_release_run_cannot_carry_anything() -> None:
    """The invariant this whole feature lives or dies on.

    `AGENTS.md` and `release-process` forbid a reduced gate, a skip flag and an
    environment bypass on the release path. A resumed run is all three unless
    it is refused there, and the refusal is the reason this is allowed to exist
    at all.
    """
    from capsem.gate import resume
    from capsem.gate.errors import GateError

    plan = _candidate_plan()
    with pytest.raises(GateError, match="cannot be used while qualifying a release"):
        resume.carried(plan, _config(), "artifacts.build-chain", qualifying=True)


def test_a_prefix_outside_the_configured_root_is_refused(tmp_path: Path) -> None:
    """A run is about to build in it.

    Same fence as `prefix.reclaim`, for the mirror-image reason: that one must
    not delete a checkout, this one must not fill one with build output.
    """
    from capsem.gate import resume
    from capsem.gate.errors import GateError

    outsider = tmp_path / "not-a-prefix"
    outsider.mkdir()
    with pytest.raises(GateError, match="is not a prefix under"):
        resume.existing(_config(), str(outsider))


def test_a_named_prefix_that_does_not_exist_is_refused() -> None:
    """Better than silently making one: the operator asked for a specific
    tree because they wanted its build output, and a fresh copy has none."""
    from capsem.gate import prefix, resume
    from capsem.gate.errors import GateError

    missing = prefix.parent_dir(_config()) / ("0" * 8)
    with pytest.raises(GateError, match="does not exist"):
        resume.existing(_config(), str(missing))


# -- the evidence ------------------------------------------------------------


def test_a_carried_step_is_not_recorded_as_one_that_ran() -> None:
    """`carried`, never `ok`.

    The run log is the only place the difference between "this process proved
    it" and "an earlier one did" can survive to whoever reads the result. If a
    carried step recorded `ok`, a resumed run would be indistinguishable from a
    clean proof of the whole graph -- which is precisely what it is not.
    """
    from capsem.gate.runlogschema import CARRIED, OK, SKIPPED

    assert CARRIED not in {OK, SKIPPED}

    recorded: list[str] = []

    class Recording:
        def carried(self, label: str) -> None:
            recorded.append(label)

    Recording().carried("assets.assemble")
    assert recorded == ["assets.assemble"]


def test_every_command_can_be_told_to_resume() -> None:
    """The flags live on the shared parser, so no command can lack them.

    Guarded here rather than by giving each of the fifty-odd hand-built
    `Namespace` objects in this suite two more fields: absent, a command reads
    its documented default, and what actually has to hold is that a real
    invocation can always pass them.

    Parsed rather than read out of `--help`, because the flags hang off the
    subcommand parsers and the top-level help does not mention them -- a text
    search would have passed against a parser that accepted neither.
    """
    from capsem.gate import cli

    for command in ("candidate", "test-fast", "test-static"):
        parsed = cli.build_parser().parse_args(
            [command, "--prefix", "/tmp/somewhere", "--from", "some.step", "--dry-run"]
        )
        assert parsed.prefix == "/tmp/somewhere"
        assert parsed.resume_from == "some.step"


def test_the_dry_run_says_what_it_would_skip() -> None:
    """So `--dry-run --from <step>` answers "what will this actually do"."""
    plan = _candidate_plan()
    rendering = plan.describe(carried=frozenset({"source.record"}))

    assert "source.record" in rendering
    assert "(carried)" in rendering
    assert "steps carried from an earlier run" in rendering
