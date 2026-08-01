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

from . import hostpackage, imagebuild, initrd, testmodules, vmmodules
from .actions import Run, Script
from .config import GateConfig
from .execution import Step, step
from .plan import Plan
from .sourcestate import RecordSourceState, RequireSourceUnchanged
from .storage import Storage


def compose(plan: Plan, config: GateConfig) -> None:
    """Every phase of the gate, in the order each depends on the last."""
    recorded = plan.add(step("source.record", RecordSourceState()))

    contracts = testmodules.release_contracts(plan, config, after=(recorded,))
    fast = testmodules.fast(plan, config, after=(contracts,))

    prepared = _prepare(plan, config, after=(fast,))
    static = testmodules.static(plan, config, after=(prepared,))
    artifacts = vmmodules.artifacts(plan, config, after=(static,))
    functional = vmmodules.functional(plan, config, after=(artifacts,))
    glowup = vmmodules.glowup(plan, config, after=(functional,))

    recipes = plan.add(
        step("recipes", Run(config.candidate.recipe_suite)), after=(glowup,)
    )
    plan.add(step("source.verify", RequireSourceUnchanged()), after=(recipes,))


def _prepare(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> Step:
    """Everything the expensive phases assume is already true.

    The benchmark recordings are cleared exactly once, here, before any module
    runs. Clearing them per module is what left a fortnight of full gates with
    an empty directory and froze the published arm64 history.
    """
    from .fileactions import Remove

    settings = config.candidate
    phase = plan.phase("prepare")

    bootstrapped = phase.add(
        step(
            "bootstrap",
            Run(["sh", str(config.path(settings.bootstrap_script)), "-y"]),
            Run(["bash", config.doctor.common_script]),
        ),
        after=after,
    )
    bounded = phase.add(
        step(
            "storage-budget",
            _release("candidate-boundary"),
            _ensure_space(config),
            Remove(config.path(config.workspace.benchmark_root)),
            contends=(config.exclusive("docker_daemon"),),
        ),
        after=(bootstrapped,),
    )
    cleaned = phase.add(
        step(
            "clean-stale",
            Script(settings.clean_stale_script),
            Run(["bash", settings.generated_settings_script, str(config.root)]),
        ),
        after=(bounded,),
    )
    return _runtime(plan, config, after=(cleaned,))


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
        step("materialize-config", Run(["bash", settings.materialize_script])),
        after=(packed,),
    )
    return phase.add(hostpackage.sign_step(config), after=(materialised,))


def _release(phase: str):
    from .actions import Call

    return Call(
        f"release the storage held after {phase}",
        lambda ctx: Storage(ctx.runner).release(phase),
    )


def _ensure_space(config: GateConfig):
    from .actions import Call

    settings = config.candidate
    return Call(
        "refuse to start a gate the daemon has no room to finish",
        lambda ctx: Storage(ctx.runner).ensure_space(*settings.candidate_budget),
    )
