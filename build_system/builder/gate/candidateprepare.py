"""Preparation graph for the complete local qualification."""

from __future__ import annotations

from . import host, hostpackage, imagebuild, initrd
from .actions import Call, Run, Script
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .fileactions import Remove
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan
from .storage import Storage


def prepare(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> Step:
    """Establish everything the expensive candidate phases assume.

    The benchmark recordings are cleared exactly once, here, before any module
    runs. Clearing them per module is what left a fortnight of full gates with
    an empty directory and froze the published arm64 history.
    """
    settings = config.candidate
    phase = plan.phase("prepare")

    # Both doctor passes run with the checks that would fail on the very thing
    # this gate is about to build turned off. Assets and guest binaries are
    # build output, and their nested fix would try to reacquire this gate's
    # machine lock.
    doctor_env = dict(settings.doctor_skips)
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
    prepared = bootstrapped
    if host.on_macos():
        prepared = phase.add(
            step(
                "tart-readiness",
                Script(
                    config,
                    settings.tart_readiness_script,
                    "--require-cache",
                    outside_sandbox=True,
                ),
                contends=(config.exclusive("apple_vz"),),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.DISK, Needs.NETWORK}),
                speed=Speed.SLOW,
            ),
            after=(bootstrapped,),
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
        after=(prepared,),
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
    # The generated-settings check compiles capsem-admin, so it is a separate
    # measured compile step rather than hidden inside cleanup.
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
    """Build, materialize, and sign the runtime consumed by later phases."""
    phase = plan.phase("prepare")
    settings = config.candidate

    assets = imagebuild.check_assets(plan, config, after=after)
    packed = initrd.pack(plan, config, after=assets)
    materialised = phase.add(
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
    built = phase.add(hostpackage.build_step(config), after=(materialised,))
    return phase.add(hostpackage.sign_step(config), after=(built,))


def _ensure_space(config: GateConfig) -> Call:
    """Refuse a gate the daemon has no room to finish."""
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
