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

from . import hostpackage, imagebuild, initrd, storage, testmodules, vmmodules
from .actions import Call, Run, Script
from .config import GateConfig
from .execution import Step, step
from .plan import Plan
from .qualification import Qualification
from .sourcestate import (
    RecordSourceState,
    RequireIsolatedBytecode,
    RequireSourceUnchanged,
)
from .storage import Storage


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
    recorded = plan.add(
        step(
            "source.record",
            # Two claims, in the order they matter: this process is not running
            # last version's bytecode, and here is the tree it is running.
            RequireIsolatedBytecode(),
            RecordSourceState(),
            # What the release guard reads back to prove the tested tree is the
            # pushed tree.
            produces=(config.path(config.candidate.source_state_file),),
        ),
        after=after,
    )

    contracts = testmodules.release_contracts(plan, config, after=(recorded,))
    fast = testmodules.fast(plan, config, after=(contracts,))
    modules = compose_modules(plan, config, qualification=qualification, after=(fast,))

    return plan.add(step("source.verify", RequireSourceUnchanged()), after=(modules,))


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
    static = testmodules.static(plan, config, after=(prepared,))
    artifacts = vmmodules.artifacts(plan, config, qualification=qualification, after=static)
    functional = vmmodules.functional(
        plan, config, qualification=qualification, after=(artifacts,)
    )
    glowup = vmmodules.glowup(plan, config, qualification=qualification, after=(functional,))

    return plan.add(step("recipes", Run(config.candidate.recipe_suite)), after=(glowup,))


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
            storage.release_action("candidate-boundary"),
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


def _ensure_space(config: GateConfig):
    """Refuse to start a gate the daemon has no room to finish.

    The release beside it is `storage.release_action`, which is the same
    spelling `storage.release_step` uses -- one place decides what releasing a
    boundary means.
    """
    settings = config.candidate
    return Call(
        "refuse to start a gate the daemon has no room to finish",
        lambda ctx: Storage(ctx.runner).ensure_space(*settings.candidate_budget),
    )
