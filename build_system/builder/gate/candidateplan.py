"""The complete local qualification, as one graph.

`just test-full` used to be a tree of processes: `candidate` ran `just _test-fast`,
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
    hostpackage,
    imagebuild,
    initrd,
    module_contracts,
    module_rehearsal,
    staticmodule,
    testmodules,
    toolchain,
    vmmodules,
)
from .actions import Call, Run, Script
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan
from .qualification import Qualification
from .sourcestate import record_step, verify_step
from .storage import Storage
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
    `just test-full` and hoping.
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
    contracts = module_contracts.release_contracts(plan, config, after=fast)
    modules = compose_modules(plan, config, qualification=qualification, after=(contracts,))

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
) -> Step:
    """Everything after the fast phase: the artifacts, the VMs, the install.

    Separate from `compose` because it is independently runnable as
    `test-candidate`, which is what a developer reaches for when the fast
    checks already passed and they do not want to repeat them.
    """
    prepared = _prepare(plan, config, after=after)
    # `generated` is the fast phase's settings step when there was a fast phase.
    # `test-candidate` runs this composition alone, where there was not, so each
    # module still makes its own.
    generated = _already_issuing(plan, audits.generated_settings(config))
    bundled = _already_issuing(plan, audits.frontend_bundle(config))
    static = staticmodule.static(
        plan, config, after=(prepared,), generated=generated, bundled=bundled
    )
    artifacts = vmmodules.artifacts(plan, config, qualification=qualification, after=static)
    functional = vmmodules.functional(
        plan,
        config,
        qualification=qualification,
        after=(artifacts,),
        generated=generated,
        isolated_assets=not qualification.pulled,
    )
    glowup = vmmodules.glowup(plan, config, qualification=qualification, after=(functional,))
    # After the glow-up, not instead of it. The local lane's install proof runs
    # the package it built; this runs the same package again through the pulled
    # path a release lane takes, against a cohort resolved by digest.
    # A release lane skips it -- there it is not a rehearsal, it is the lane.
    # Asked after `functional`, because that is when the step it finds exists.
    installed = _already_issuing(plan, toolchain.node(config, config.functional.node_workspaces))
    rehearsed = module_rehearsal.rehearsal(
        plan,
        config,
        qualification=qualification,
        after=(glowup,),
        generated=generated,
        node=installed,
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


def _prepare(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> Step:
    """Everything the expensive phases assume is already true.

    The benchmark recordings are cleared exactly once, here, before any module
    runs. Clearing them per module is what left a fortnight of full gates with
    an empty directory and froze the published arm64 history.
    """
    from .fileactions import Remove

    settings = config.candidate
    phase = plan.phase("prepare")

    # Both doctor passes run with the checks that would fail on the very thing
    # this gate is about to build turned off -- the same pair `imagebuild`
    # already passes for the same reason. `assets/` and the guest binaries are
    # build output, so a run that does not inherit a warm checkout has neither,
    # and doctor was reporting `manifest.json missing` about a manifest
    # `assets.assemble` produces sixty steps later.
    #
    # Skipping the *check* rather than letting the fix run: the fix is
    # `just _build-assets`, which takes the machine lock this run is holding.
    doctor_env = dict(config.imagebuild.doctor_skips)
    bootstrapped = phase.add(
        step(
            "bootstrap",
            Run(["sh", str(config.path(settings.bootstrap_script)), "-y"], env=doctor_env),
            Run(["bash", config.doctor.common_script], env=doctor_env),
            kind=Kind.COMPILE,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=after,
    )
    bounded = phase.add(
        step(
            "storage-budget",
            _ensure_space(config),
            Remove(config.path(config.workspace.benchmark_root)),
            contends=(config.exclusive("docker_daemon"),),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=(bootstrapped,),
    )
    cleaned = phase.add(
        step(
            "clean-stale",
            Script(config, settings.clean_stale_script),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=(bounded,),
    )
    # Its own step, and not because deleting stale files and checking the
    # generated settings are unrelated -- though they are. The check runs
    # `generate-settings.sh`, which runs `cargo run -p capsem-core`, so a step
    # named for a cleanup was building Rust and claiming nothing while it did.
    # One step is one measurement, and this one was measuring two things.
    checked = phase.add(
        step(
            "verify-generated-settings",
            Run(["bash", settings.generated_settings_script, str(config.root)]),
            contends=(config.exclusive("workspace_binaries"),),
            kind=Kind.COMPILE,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=(cleaned,),
    )
    return _runtime(plan, config, after=(checked,))


def _runtime(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> Step:
    """`_prepared-runtime`: the assets, the initrd, and the generated config.

    Three recipes that were `just` dependencies of one another. As edges the
    order is the same and the reason is visible: the initrd is repacked against
    assets that must already exist, and the service reads config materialised
    from them.
    """
    phase = plan.phase("prepare")
    settings = config.candidate

    assets = imagebuild.check_assets(plan, config, after=after)
    packed = initrd.pack(plan, config, after=assets)
    materialised = phase.add(
        # `COMPILE`, and claiming the workspace, because the script runs
        # `cargo run -p capsem-admin` once per profile. It reads as
        # configuration work from the label alone, which is exactly the
        # mismatch `tests/citadel/test_step_actions_are_atomic.py` refuses.
        step(
            "materialize-config",
            Run(["bash", settings.materialize_script]),
            contends=(config.exclusive("workspace_binaries"),),
            kind=Kind.COMPILE,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=(packed,),
    )
    # Built, then signed. `sign_step` codesigns `target/debug/*` and nothing
    # in this plan ever produced them -- it worked because a developer machine
    # had run `just build` at some point, and failed with `No such file or
    # directory` the first time a run got a checkout of its own.
    built = phase.add(hostpackage.build_step(config), after=(materialised,))
    return phase.add(hostpackage.sign_step(config), after=(built,))


def _ensure_space(config: GateConfig):
    """Refuse to start a gate the daemon has no room to finish.

    The second configured argument is an evidence label for the capacity
    check, not a release boundary. There is no working resource to release at
    candidate start; a release action here would only take two snapshots and
    reclaim nothing.
    """
    settings = config.candidate
    return Call(
        "refuse to start a gate the daemon has no room to finish",
        lambda ctx: Storage(ctx.runner).ensure_space(*settings.candidate_budget),
        justification=CallJustification(
            kind=OpaqueKind.PURE_INSPECTION,
            reason="reads the daemon's free space and refuses a gate it has no room to finish",
            effects=machine_effects(Effect.PROCESS),
        ),
    )
