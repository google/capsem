"""Preparation graph for the complete local qualification."""

from __future__ import annotations

from . import bench, host, hostpackage, imagebuild, initrd, packagepreflight
from .actions import Call, Run, Script
from .cachecontrol import CacheControl
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .fileactions import Remove
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan


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
            "cache-enforcement",
            _enforce_cache(config),
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
    harness, fitness = bench.fitness(config)
    built_harness = phase.add(harness, after=(checked,))
    fit = phase.add(fitness, after=(built_harness,))
    dependencies = packagepreflight.fragment(plan, config, after=(fit,))
    return _runtime(plan, config, after=(dependencies,))


def _runtime(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> Step:
    """Build, materialize, and sign the runtime consumed by later phases."""
    phase = plan.phase("prepare")

    assets = imagebuild.check_assets(
        plan,
        config,
        after=after,
        doctor_skips=dict(config.candidate.doctor_skips),
    )
    packed = initrd.pack(plan, config, after=assets)
    materialised = phase.add(materialize_config_step(config), after=(packed,))
    built = phase.add(hostpackage.build_step(config), after=(materialised,))
    return phase.add(hostpackage.sign_step(config), after=(built,))


def materialize_config_step(config: GateConfig) -> Step:
    """Produce the canonical config half of locally built profile content."""
    return step(
        "materialize-config",
        Run(["bash", config.candidate.materialize_script]),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    )


def _enforce_cache(config: GateConfig) -> Call:
    """Enforce the repository cache maximum before expensive work."""
    settings = config.candidate
    return Call(
        "enforce configured cache maximums before expensive work",
        lambda ctx: CacheControl(ctx.runner).enforce(settings.candidate_cache, "candidate"),
        justification=CallJustification(
            kind=OpaqueKind.RUNTIME_DERIVED,
            reason="accounts for owned cache bytes and prunes only when a maximum is crossed",
            effects=machine_effects(Effect.PROCESS, Effect.FILESYSTEM, Effect.HOST_STATE),
        ),
    )
