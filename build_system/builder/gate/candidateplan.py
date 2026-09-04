"""The complete local qualification, as one graph.

`just test` used to be a tree of processes: `candidate` ran `just _test-fast`,
which ran `capsem-gate test-fast`; then a Colima wrapper around `just
_test-candidate`, which ran `capsem-gate test-candidate`, which ran four more
gate commands, each of which ran several more. Every one of those is exclusive
and the machine lock is not reentrant, so the shape was a queue of children
waiting out a 7200-second timeout for a lock their own parent held.

Composing it moves the ordering out of three languages -- `just` dependencies,
the line order of a shell body, and four separate `plan()` methods -- and into
one set of edges. Two things that were incidental become structural on the way.

The source state is recorded by a step and re-asserted by another, so what
passed is what was measured. And the two things that must happen even when the
gate fails are `Resource`s rather than steps, because a step whose dependency
failed is skipped -- which is exactly wrong for cleanup.
"""

from __future__ import annotations

from . import (
    audits,
    candidateprepare,
    module_contracts,
    module_rehearsal,
    staticmodule,
    testmodules,
    toolchain,
    vmmodules,
    webaudits,
)
from .actions import Run
from .config import GateConfig
from .content import ProfileContent
from .execution import Kind, Speed, Step, step
from .plan import Plan
from .qualification import Qualification
from .sourcestate import record_step, verify_step
from .timingratchet import EnforceTimingRegression, TimingBoundary


def _already_issuing(plan: Plan, candidate: Step) -> Step | None:
    """A step already in this plan that issues exactly this command.

    Composition is where repetition hides. Every module owns its prerequisites
    so it can run alone, which is right, and three of them then generated the
    same file into the same prefix in one run at seventy-five seconds each.

    Asked explicitly here rather than deduplicated inside `Plan.add`: doing it
    there also merged `pnpm install` across phases and reordered the graph.

    Compared by rendered command, so nothing depends on a label spelled twice,
    and only for commands -- a `Call` renders its description, so two of them
    targeting different architectures read alike while doing different work.
    """
    kinds = frozenset({"script", "run"})
    if not candidate.actions or any(type(action).name not in kinds for action in candidate.actions):
        return None
    issued = tuple(action.render() for action in candidate.actions)
    return next(
        (
            existing
            for existing in plan.steps
            if tuple(action.render() for action in existing.actions) == issued
        ),
        None,
    )


def compose(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
) -> Step:
    """Every phase of the gate, in the order each depends on the last.

    Composed by `candidate` and, unchanged, by both release commands -- which
    is the point: a release runs the complete local proof rather than a
    reduced one, and now it runs it in the same process rather than launching
    `just test` and hoping.
    """
    recorded = plan.shared(record_step(config), after=after)

    # Cheap before expensive. These two were the other way round, and they are
    # serial either way -- so the order cost nothing in total time and
    # everything in how long a trivial failure takes to arrive. `contracts` is
    # a nine-minute pytest run, and Ruff, which answers in under two seconds,
    # sat behind it: two consecutive `release-profile` attempts died at 9m12
    # and 11m39 on one unused local variable.
    #
    # `fast` also opens with `toolchain.sync`, so running it first means the
    # contracts suite is no longer the step that discovers the environment.
    fast = testmodules.fast(plan, config, after=(recorded,))
    node = _already_issuing(plan, toolchain.node(config))
    generated = _already_issuing(plan, audits.generated_settings(config))
    contracts = module_contracts.release_contracts(
        plan,
        config,
        after=fast,
        node=node,
        generated=generated,
        seed_coverage=True,
    )
    modules = compose_modules(
        plan,
        config,
        qualification=qualification,
        after=(contracts,),
        source_contracts_proved=True,
    )

    return plan.add(
        verify_step(EnforceTimingRegression(TimingBoundary.QUALIFICATION)),
        after=(modules,),
    )


def compose_modules(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
    source_contracts_proved: bool = False,
) -> Step:
    """Everything after the fast phase: the artifacts, the VMs, the install.

    Separate from `compose` because it is independently runnable as
    `test-candidate`, which is what a developer reaches for when the fast
    checks already passed and they do not want to repeat them.
    """
    prepared = candidateprepare.prepare(plan, config, after=after)
    # `generated` is the fast phase's settings step when there was a fast phase.
    # `test-candidate` runs this composition alone, where there was not, so each
    # module still makes its own.
    generated = _already_issuing(plan, audits.generated_settings(config))
    bundled = _already_issuing(plan, webaudits.frontend_bundle(config))
    node = _already_issuing(plan, toolchain.node(config))
    static = staticmodule.static(
        plan,
        config,
        after=(prepared.ready,),
        generated=generated,
        bundled=bundled,
        node=node,
    )
    signed = next(step for step in static if step.label == "static.sign")
    artifacts = vmmodules.artifacts(
        plan,
        config,
        qualification=qualification,
        after=static,
        node=node,
        bundled=bundled,
    )
    functional = vmmodules.functional(
        plan,
        config,
        qualification=qualification,
        after=(artifacts,),
        generated=generated,
        node=node,
        signed=signed,
        source_contracts_proved=source_contracts_proved,
        isolated_assets=not qualification.pulled,
    )
    glowup = vmmodules.glowup(
        plan,
        config,
        qualification=qualification,
        after=(functional,),
        local_content=ProfileContent.built_profile(
            config,
            config.suites.pytest.base_profile,
        ),
        materialized=prepared.profile_content,
    )
    # After the glow-up, not instead of it. The local lane's install proof runs
    # the package it built; this runs the same package again through the pulled
    # path a release lane takes, against a cohort resolved by digest.
    # A release lane skips it -- there it is not a rehearsal, it is the lane.
    # Asked after `functional`, because that is when the step it finds exists.
    rehearsed = module_rehearsal.rehearsal(
        plan,
        config,
        qualification=qualification,
        after=(glowup,),
    )

    return plan.add(
        step(
            "recipes",
            Run(config.candidate.recipe_suite),
            kind=Kind.STATIC_TEST,
            speed=Speed.FAST,
        ),
        after=(rehearsed,),
    )
