"""Repacking the initrd, without corrupting every tree that shares its inode.

This is the recipe `AtomicReplace` was written for. `create_hash_assets.py`
gives the unhashed `initrd.img` a hash-named hardlink sharing one inode, so a
shell `> "$INITRD"` -- truncate and write in place -- mutates that hardlink's
contents too. A VM mid-`VmConfig::build`, reading the old hash-named path,
then sees bytes that do not match the embedded hash.

Cross-compilation runs only when staged binaries are missing or older than
their inputs; the former shell `find -newer` is a stat comparison here.
"""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

from . import crossexec, host
from .actions import Action, Run, Script
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step
from .imagebases import MaterializeRustBuilders, Prefetch
from .initrdactions import _Repack, _Stage
from .initrdpaths import initrd_target, needs_rebuild
from .plan import Plan
from .versions import workspace_version


def repack_step(config: GateConfig, targets: Mapping[str, tuple[Path, ...]]) -> Step:
    """One resumable frontier that stages and repacks exact initrd targets."""
    actions: list[Action] = []
    produced: list[Path] = []
    for spelling, paths in targets.items():
        arch = config.arch(spelling).name
        if not paths:
            continue
        actions.append(_Stage(config, arch))
        for target in paths:
            actions.append(_Repack(target=target, arch=arch))
            produced.append(target)
    if not produced:
        raise GateError("an initrd repack step needs at least one exact target")
    return step(
        "pack-initrds",
        *actions,
        contends=(config.exclusive("docker_daemon"),),
        produces=tuple(produced),
        kind=Kind.PACKAGE,
        needs=frozenset({Needs.DOCKER, Needs.DISK}),
        speed=Speed.SLOW,
    )


def finalize(
    plan: Plan,
    config: GateConfig,
    *,
    assets: Path,
    after: tuple[Step, ...],
    phase_name: str | None = None,
) -> Step:
    """Regenerate manifest and aliases after initrd bytes change."""
    phase = plan.phase(phase_name) if phase_name else plan
    manifest = phase.add(
        step(
            "manifest",
            Run(
                [
                    *config.initrd.manifest,
                    str(assets),
                    "--version",
                    workspace_version(config.root),
                ]
            ),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=after,
    )
    return phase.add(
        step("hash-aliases", Script(config.initrd.hash_assets, str(assets)),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=(manifest,),
    )


class PackInitrdCommand(
    GateCommand,
    name="pack-initrd",
    help="rebuild the guest binaries if stale, then repack the initrd",
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        pack(plan, self._config)
        return plan


def pack(plan: Plan, config: GateConfig, *, after: tuple = ()) -> Step:
    """Rebuild the guest binaries if stale, then repack the initrd."""
    phase = plan.phase("initrd")
    settings = config.initrd

    previous: tuple = after
    if needs_rebuild(config):
        if host.on_macos():
            arch = config.host_arch().name
            base = phase.add(
                step(
                    "guest-base",
                    Prefetch((arch,), rust_names=(arch,)),
                    contends=(config.exclusive("docker_daemon"),),
                    kind=Kind.PACKAGE,
                    needs=frozenset({Needs.DOCKER, Needs.DISK}),
                    speed=Speed.SLOW,
                ),
                after=after,
            )
            execution = phase.add(
                step(
                    "guest-execution",
                    crossexec.Require((arch,)),
                    contends=(config.exclusive("docker_daemon"),),
                    kind=Kind.PACKAGE,
                    needs=frozenset({Needs.DOCKER, Needs.DISK}),
                    speed=Speed.SLOW,
                ),
                after=(base,),
            )
            previous = (
                phase.add(
                    step(
                        "guest-builder",
                        MaterializeRustBuilders((arch,)),
                        contends=(config.exclusive("docker_daemon"),),
                        kind=Kind.PACKAGE,
                        needs=frozenset({Needs.DOCKER, Needs.DISK}),
                        speed=Speed.SLOW,
                    ),
                    after=(execution,),
                ),
            )
        previous = (
            phase.add(
                step(
                    "guest-agents",
                    Run([*settings.build, "--arch", config.host_arch().name]),
                    contends=(config.exclusive("docker_daemon"),),
                    kind=Kind.PACKAGE,
                    needs=frozenset({Needs.DOCKER, Needs.DISK}),
                    speed=Speed.SLOW,
                ),
                after=previous,
            ),
        )

    packed = phase.add(
        step(
            "repack",
            _Repack(),
            # The initrd the VM boots. The one artifact whose contents decide
            # whether a guest runs the code this gate just built.
            produces=(initrd_target(config),),
            kind=Kind.PACKAGE,
            needs=frozenset({Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=previous,
    )

    return finalize(
        plan,
        config,
        assets=config.path(config.imagebuild.output),
        after=(packed,),
        phase_name="initrd",
    )
