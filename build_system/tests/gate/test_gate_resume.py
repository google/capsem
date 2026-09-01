"""Continuing a failed run instead of replaying it.

Six consecutive `just test` runs were spent proving the same twenty minutes of
work to reach a new failure one step further on. The private checkout made that
worse, not better: a fresh copy per run starts with no `cache/target/`, so every
replay is cold.

Working-tree `--prefix <tree> --from <step>` is diagnostic continuation and
never qualification. Exact-commit `just test` instead selects its retained
full-SHA prefix and deepest frontier from archived event evidence; a resumed
journal names its content-addressed parent. Both modes record carried steps as
`carried`, and release commands still reject continuation flags outright.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def _config():
    from capsem_builder.gate import config as gate_config

    return gate_config.load(PROJECT_ROOT)


def _candidate_plan():
    # Through the helper, which is the one place that knows importing `cli` is
    # what fills the registry. Spelling that import here needed a suppression
    # for a name nothing reads.
    from capsem_builder.gate.qualification import from_environment
    from helpers.gate import built_command

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
    from capsem_builder.gate import resume

    plan = _candidate_plan()
    carried = resume.ancestors(plan, "artifacts.build-chain")

    assert "source.record" in carried, "the earliest step is an ancestor of nearly everything"
    assert "artifacts.build-chain" not in carried, "the named step runs; that is the point"
    assert "source.verify" not in carried, "a step that comes after must never be carried"

    # And it is transitively closed: every ancestor's own ancestors are in too,
    # or the runner would block forever on a dependency nobody satisfied.
    for label in carried:
        assert plan.after_of(label) <= carried, f"{label} is carried but its inputs are not"


def test_a_refreshed_prefix_reruns_source_identity_before_carried_work() -> None:
    """Refreshing source invalidates the old run's identity receipt.

    The retained prefix is refreshed before the child gate starts.  Carrying
    ``source.record`` after that refresh left the old receipt in ``target``;
    the entire resumed candidate then passed before ``source.verify`` rejected
    the revision mismatch at the final step.  Resume policy belongs to the
    step, not a label exception in the resolver, so any future identity
    boundary can make the same declaration.
    """
    from capsem_builder.gate import resume
    from capsem_builder.gate.execution import ResumePolicy

    plan = _candidate_plan()
    carried = resume.carried(
        plan,
        _config(),
        "artifacts.build-chain",
        qualifying=False,
    )

    assert plan.step_named("source.record").resume is ResumePolicy.ALWAYS_RUN
    assert "source.record" not in carried
    assert carried == resume.ancestors(plan, "artifacts.build-chain") - {"source.record"}


def test_always_run_resume_policy_executes_before_carried_dependants() -> None:
    """A non-carried ancestor remains a real graph dependency.

    Filtering the carried set must not make the scheduler start the frontier
    before the refreshed receipt exists.  The graph still orders the always-
    run step; only its downstream reusable work is skipped.
    """
    from capsem_builder.gate.actions import Action
    from capsem_builder.gate.context import Context
    from capsem_builder.gate.execution import ResumePolicy, step
    from capsem_builder.gate.plan import Plan
    from helpers.gate import RecordingRunner

    seen: list[str] = []

    class Record(Action, name="record-resume-policy-action"):
        def __init__(self, label: str) -> None:
            self._label = label

        def render(self) -> str:
            return f"record {self._label}"

        def perform(self, context: Context) -> None:
            del context
            seen.append(self._label)

    plan = Plan("resume-policy")
    identity = plan.add(
        step(
            "identity",
            Record("identity"),
            resume=ResumePolicy.ALWAYS_RUN,
        )
    )
    reused = plan.add(
        step("reused", Record("reused")),
        after=(identity,),
    )
    plan.add(
        step("frontier", Record("frontier")),
        after=(reused,),
    )

    config = _config()
    plan.run(
        Context(
            RecordingRunner(config.root),
            config,
            carried=frozenset({"reused"}),
        )
    )

    assert seen == ["identity", "frontier"]


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
    from capsem_builder.gate import resume
    from capsem_builder.gate.installimage import InstallImageStep

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
    from capsem_builder.gate import resume
    from capsem_builder.gate.errors import GateError

    plan = _candidate_plan()
    with pytest.raises(GateError, match=re.escape("artifacts.build-chain")):
        resume.ancestors(plan, "build-chain")


def test_carrying_nothing_is_the_default() -> None:
    """A run without `--from` proves the whole graph."""
    from capsem_builder.gate import resume

    plan = _candidate_plan()
    assert resume.carried(plan, _config(), None, qualifying=False) == frozenset()


# -- the refusals that keep it from being a skip flag ------------------------


def test_release_qualification_refuses_a_frontier_derived_only_from_graph_shape() -> None:
    """A release attempt has no recursively verified continuation lineage.

    Candidate qualification validates its requested frontier against archived
    exact-source evidence in ``qualificationflow.decide``. Public dispatch and
    release CI cannot make that claim: deriving their carried work from graph
    shape alone could skip qualification acceptance or a fresh mutable channel
    fetch. They must run every release-attempt edge.
    """
    from capsem_builder.gate import resume
    from capsem_builder.gate.errors import GateError

    plan = _candidate_plan()
    with pytest.raises(GateError, match="--from cannot be used while qualifying a release"):
        resume.carried(plan, _config(), "artifacts.build-chain", qualifying=True)

    assert resume.carried(
        plan, _config(), "artifacts.build-chain", qualifying=False
    ), "local candidate qualification validates this carry against its journal later"


def test_scratch_carries_nothing_however_much_is_proven() -> None:
    """The escape hatch, for when a local pass must mean a cold pass."""
    from capsem_builder.gate import resume
    from capsem_builder.gate.errors import GateError

    plan = _candidate_plan()
    assert resume.carried(plan, _config(), resume.SCRATCH, qualifying=False) == frozenset()
    with pytest.raises(GateError, match="--from cannot be used while qualifying a release"):
        resume.carried(plan, _config(), resume.SCRATCH, qualifying=True)


def test_auto_leaves_the_frontier_to_the_gate() -> None:
    """`--from` is never empty; `auto` means "you choose".

    Resume has to be the default, or it is a flag people remember only after
    paying for not remembering it.
    """
    from capsem_builder.gate import resume

    plan = _candidate_plan()
    assert resume.carried(plan, _config(), resume.AUTO, qualifying=False) == frozenset()
    assert resume.carried(plan, _config(), resume.AUTO, qualifying=True) == frozenset()
    assert resume.explicit(resume.AUTO) is None
    assert resume.explicit(resume.SCRATCH) == resume.SCRATCH
    assert resume.explicit("artifacts.build-chain") == "artifacts.build-chain"


def test_a_prefix_outside_the_configured_root_is_refused(tmp_path: Path) -> None:
    """A run is about to build in it.

    Same fence as `prefix.reclaim`, for the mirror-image reason: that one must
    not delete a checkout, this one must not fill one with build output.
    """
    from capsem_builder.gate import resume
    from capsem_builder.gate.errors import GateError

    outsider = tmp_path / "not-a-prefix"
    outsider.mkdir()
    with pytest.raises(GateError, match="is not a prefix under"):
        resume.existing(_config(), str(outsider))


def test_a_named_prefix_that_does_not_exist_is_refused() -> None:
    """Better than silently making one: the operator asked for a specific
    tree because they wanted its build output, and a fresh copy has none."""
    from capsem_builder.gate import prefix, resume
    from capsem_builder.gate.errors import GateError

    missing = prefix.parent_dir(_config()) / ("0" * 8)
    with pytest.raises(GateError, match="does not exist"):
        resume.existing(_config(), str(missing))


def test_a_named_prefix_moves_a_focused_command_into_that_checkout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The shared flag is an execution contract, not candidate-only syntax.

    Cross-compile accepted a retained prefix and its selected content, then
    wrote the package and run journal into the source checkout because the
    command does not request a fresh private checkout.  A caller naming an
    existing prefix is different: every compatible focused command must run
    there so its products and evidence have one location.
    """
    from capsem_builder.gate import prefix, resume
    from capsem_builder.gate.plan import Plan
    from helpers.gate import built_command

    reused = tmp_path / "retained-prefix"
    reused.mkdir()
    command = built_command(
        PROJECT_ROOT,
        "cross-compile",
        (
            ("arch", "x86_64"),
            ("content_root", None),
            ("defer_proof", False),
            ("prefix", str(reused)),
            ("resume_from", None),
        ),
    )
    monkeypatch.setattr(command, "plan", lambda: Plan(command.name))
    monkeypatch.setattr(resume, "resolve", lambda *args, **kwargs: (frozenset(), reused))
    monkeypatch.setattr(prefix, "source_checkout", lambda _config: None)
    entered: list[Path | None] = []

    def run_in_prefix(
        _runner, _config, _arguments, *, commit=None, reuse=None, clean=False
    ) -> int:
        assert commit is None
        entered.append(reuse)
        return 0

    monkeypatch.setattr(prefix, "run_from_private_copy", run_in_prefix)
    # This test models a new top-level focused invocation.  The complete gate
    # also runs this contract, so do not let its inherited lock marker turn the
    # simulated invocation into the nested-gate case guarded by preflight.
    monkeypatch.delenv(command._config.locks.gate.run_marker, raising=False)

    with pytest.raises(SystemExit) as stopped:
        command.execute()

    assert stopped.value.code == 0
    assert entered == [reused]


# -- the evidence ------------------------------------------------------------


def test_a_carried_step_is_not_recorded_as_one_that_ran() -> None:
    """`carried`, never `ok`.

    The run log is the only place the difference between "this process proved
    it" and "an earlier one did" can survive to whoever reads the result. If a
    carried step recorded `ok`, a resumed run would be indistinguishable from a
    clean proof of the whole graph -- which is precisely what it is not.
    """
    from capsem_builder.gate.runlogschema import CARRIED, OK, SKIPPED

    assert CARRIED not in {OK, SKIPPED}

    recorded: list[str] = []

    class Recording:
        def carried(self, label: str) -> None:
            recorded.append(label)

    Recording().carried("assets.assemble")
    assert recorded == ["assets.assemble"]


def test_a_carried_runtime_dependency_is_checked_before_any_plan_work() -> None:
    """A retained prefix cannot retain a Docker image another rail reclaimed."""
    from capsem_builder.gate.actions import Action
    from capsem_builder.gate.context import Context
    from capsem_builder.gate.errors import GateError
    from capsem_builder.gate.execution import Requires, step
    from capsem_builder.gate.plan import Plan
    from helpers.gate import RecordingJournal, RecordingRunner

    ran: list[str] = []

    class Missing(Action, name="missing-carried-product"):
        def render(self) -> str:
            return "require the carried helper image"

        def perform(self, context: Context) -> None:
            del context
            raise GateError("exact helper image is absent")

    class Work(Action, name="work-after-carried-product"):
        def render(self) -> str:
            return "consume the carried helper image"

        def perform(self, context: Context) -> None:
            del context
            ran.append("ran")

    plan = Plan("resume-product")
    materialized = plan.add(step("materialize", carry_checks=(Missing(),)))
    plan.add(
        step("consume", Work()),
        after=(materialized,),
        requires=Requires.ARTIFACT,
    )
    context = Context(
        RecordingRunner(PROJECT_ROOT),
        _config(),
        journal=RecordingJournal(),
        carried=frozenset({"materialize"}),
    )

    with pytest.raises(
        GateError,
        match=r"cannot carry 'materialize'.*exact helper image is absent.*--from materialize",
    ):
        plan.run(context)

    assert ran == []


def test_a_carried_runtime_dependency_released_after_its_consumers_is_not_checked() -> None:
    """Bounded cleanup may reclaim an intermediate after its last real use.

    A late diagnostic frontier still descends from that producer through the
    plan's ordering graph, but it does not consume the producer's bytes.  If
    the one declared artifact consumer was carried, requiring the deliberately
    reclaimed product makes an otherwise valid retained prefix impossible to
    resume.
    """
    from capsem_builder.gate.actions import Action
    from capsem_builder.gate.context import Context
    from capsem_builder.gate.execution import Requires, step
    from capsem_builder.gate.plan import Plan
    from helpers.gate import RecordingJournal, RecordingRunner

    checked: list[str] = []
    ran: list[str] = []

    class Reclaimed(Action, name="reclaimed-carried-product"):
        def render(self) -> str:
            return "require a deliberately reclaimed helper image"

        def perform(self, context: Context) -> None:
            del context
            checked.append("checked")

    class Work(Action, name="work-after-reclaimed-product"):
        def render(self) -> str:
            return "run work unrelated to the reclaimed helper image"

        def perform(self, context: Context) -> None:
            del context
            ran.append("ran")

    plan = Plan("resume-after-product-lifetime")
    materialized = plan.add(step("materialize", carry_checks=(Reclaimed(),)))
    consumed = plan.add(
        step("consume"),
        after=(materialized,),
        requires=Requires.ARTIFACT,
    )
    plan.add(step("later", Work()), after=(consumed,))
    context = Context(
        RecordingRunner(PROJECT_ROOT),
        _config(),
        journal=RecordingJournal(),
        carried=frozenset({"materialize", "consume"}),
    )

    plan.run(context)

    assert checked == []
    assert ran == ["ran"]


def test_a_carried_check_without_artifact_consumers_remains_fail_closed() -> None:
    """Durable evidence checks are never inferred to have expired."""
    from capsem_builder.gate.actions import Action
    from capsem_builder.gate.context import Context
    from capsem_builder.gate.errors import GateError
    from capsem_builder.gate.execution import step
    from capsem_builder.gate.plan import Plan
    from helpers.gate import RecordingJournal, RecordingRunner

    class Missing(Action, name="missing-durable-evidence"):
        def render(self) -> str:
            return "require durable evidence"

        def perform(self, context: Context) -> None:
            del context
            raise GateError("durable evidence is missing")

    plan = Plan("resume-durable-evidence")
    plan.add(step("evidence", carry_checks=(Missing(),)))
    context = Context(
        RecordingRunner(PROJECT_ROOT),
        _config(),
        journal=RecordingJournal(),
        carried=frozenset({"evidence"}),
    )

    with pytest.raises(GateError, match="durable evidence is missing"):
        plan.run(context)


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
    from capsem_builder.gate import cli

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


def test_until_carries_a_step_and_everything_after_it() -> None:
    """The mirror of `--from`, and the reason a local replay is safe.

    The installed-package proof purges the host `capsem`, deletes `~/.capsem`
    and reinstalls from the channel under test. That is correct on a runner
    deleted minutes later and destructive on a machine somebody is working on --
    which is what happened the first time the release lane was replayed here.
    """
    from argparse import Namespace

    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import resume
    from helpers.gate import gate_plan

    plan = gate_plan("candidate")
    config = gate_config.load(PROJECT_ROOT)

    carried, _ = resume.resolve(
        plan, config, Namespace(stop_before="glowup.install"), qualifying=False
    )

    assert "glowup.install" in carried, "the named step is carried, not run"
    assert "rehearsal.cohort" in carried, "and so is everything waiting on it"
    assert "fast.citadel" not in carried, "everything before it still runs"


def test_a_release_refuses_until_for_the_reason_it_refuses_from() -> None:
    """Both drop steps this process did not execute."""
    from argparse import Namespace

    import pytest
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import resume
    from capsem_builder.gate.errors import GateError
    from helpers.gate import gate_plan

    with pytest.raises(GateError, match="--until cannot be used while qualifying"):
        resume.resolve(
            gate_plan("candidate"),
            gate_config.load(PROJECT_ROOT),
            Namespace(stop_before="glowup.install"),
            qualifying=True,
        )


def test_a_release_refuses_a_named_prefix_even_with_the_default_frontier() -> None:
    """A retained directory is continuation authority only for candidate evidence."""
    from argparse import Namespace

    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import resume
    from capsem_builder.gate.errors import GateError
    from helpers.gate import gate_plan

    with pytest.raises(GateError, match="--prefix cannot be used while qualifying a release"):
        resume.resolve(
            gate_plan("candidate"),
            gate_config.load(PROJECT_ROOT),
            Namespace(prefix="/tmp/not-evidence", resume_from=resume.AUTO),
            qualifying=True,
        )


def test_until_names_the_step_and_the_plan_it_is_missing_from() -> None:
    """`glowup.package` is in the release plan and not the candidate one.

    Naming both is what turns a typo into a two-second fix: the replay tool
    defaults to that label, and pointed at the wrong plan it must say which.
    """
    from argparse import Namespace

    import pytest
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import resume
    from capsem_builder.gate.errors import GateError
    from helpers.gate import gate_plan

    with pytest.raises(GateError, match=r"no step named 'glowup.package' in the candidate plan"):
        resume.resolve(
            gate_plan("candidate"),
            gate_config.load(PROJECT_ROOT),
            Namespace(stop_before="glowup.package"),
            qualifying=False,
        )
