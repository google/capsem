"""Recover this host's VM assets: the gate's precondition, decided once per run.

Split from `imagebuild`, which owns building assets on request; this module
owns deciding whether the gate needs to, and it is where the `--slow`
refusal lives.
"""

from __future__ import annotations

from dataclasses import replace

from . import assetdependencies, crossexec, imagebases, imagebuild
from .assetcondition import AssetRecovery
from .command import GateCommand
from .config import GateConfig
from .execution import Kind, Speed, Step, step
from .fileactions import Remove
from .imagedoctor import doctor
from .plan import Plan
from .rebuildpermission import DEFAULT_PERMISSION, RebuildPermission


class CheckAssetsCommand(
    GateCommand,
    name="check-assets",
    help="build this host's VM assets if they are not already there",
):
    """The gate's precondition, using config's one architecture mapping."""

    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        check_assets(plan, self._config, permission=self.rebuild_permission)
        return plan


def check_assets(
    plan: Plan,
    config: GateConfig,
    *,
    permission: RebuildPermission = DEFAULT_PERMISSION,
    after: tuple[Step, ...] = (),
    doctor_skips: dict[str, str] | None = None,
) -> tuple[Step, ...]:
    """Build this host's VM assets if they are not already there.

    The graph is invariant: each action checks asset presence only when it
    executes. A warm checkout therefore does no recovery work without hiding
    labels that the private prefix may need to resume.
    """
    arch = config.host_arch()
    recovery = AssetRecovery(config, arch, permission)
    phase = plan.phase("assets")
    names = (arch.name,)
    rust_builders = imagebases.required_rust_builder_names(config, names)
    bases = phase.add(
        _when_stale(
            recovery,
            step(
                "base-images",
                imagebases.Prefetch(names, rust_names=rust_builders, asset_tools=True),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.PACKAGE,
                needs=imagebuild.PULLS,
                speed=Speed.SLOW,
            ),
        ),
        after=after,
    )
    checked = phase.add(
        _when_stale(recovery, doctor(config, skips=doctor_skips)),
        after=(bases,),
    )
    ready = phase.add(
        _when_stale(
            recovery,
            step(
                "guest-execution",
                crossexec.Require(names),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.PACKAGE,
                needs=imagebuild.BUILDS,
                speed=Speed.SLOW,
            ),
        ),
        after=(checked,),
    )
    if rust_builders:
        # Not `_when_stale`. The warm-asset shortcut is about not rebuilding
        # assets; this builds the *tool* that builds them, and
        # `initrd.guest-agents` runs `capsem-builder agent` without asking
        # whether any asset is present. The builder also goes stale on its own
        # schedule -- it is keyed on `Cargo.lock`, so one added dependency
        # invalidates it while every asset on disk stays valid, which is the
        # case the condition cannot see. A run then skipped materialising it
        # and failed four steps later with "locked guest Rust builder is
        # missing".
        #
        # Unconditional costs nothing warm: `materialize_rust_builders` checks
        # `image_exists` and notes that it is already there.
        ready = phase.add(
            step(
                "guest-builders",
                imagebases.MaterializeRustBuilders(rust_builders),
                contends=(config.exclusive("docker_daemon"),),
                carry_checks=(imagebases.RequireRustBuilders(rust_builders),),
                kind=Kind.PACKAGE,
                needs=imagebuild.BUILDS,
                speed=Speed.SLOW,
            ),
            after=(ready,),
        )
    ready = phase.add(
        _when_stale(
            recovery,
            step(
                "asset-tools",
                imagebases.MaterializeAssetTools(),
                contends=(config.exclusive("docker_daemon"),),
                carry_checks=(imagebases.RequireAssetTools(),),
                kind=Kind.PACKAGE,
                needs=imagebuild.BUILDS,
                speed=Speed.SLOW,
            ),
        ),
        after=(ready,),
    )
    ready = phase.add(
        _when_stale(
            recovery,
            assetdependencies.dependency_step(
                config, imagebuild.profiles(config), names, label="recovery-dependencies"
            ),
        ),
        after=(ready,),
    )
    images: list[Step] = []
    manifest = config.path(config.imagebuild.output) / config.install.manifest_name
    for profile in imagebuild.profiles(config):
        subject = imagebuild.build(config, profile=profile, arch=arch.name, template="all")
        subject = replace(subject, actions=(Remove(manifest), *subject.actions))
        ready = phase.add(_when_stale(recovery, subject), after=(ready,))
        images.append(ready)
    phase.add(recovery.record_step(), after=(ready,))
    return tuple(images)


def _when_stale(recovery: AssetRecovery, subject: Step) -> Step:
    return replace(
        subject,
        actions=tuple(recovery.when(action) for action in subject.actions),
        carry_checks=tuple(recovery.when(check) for check in subject.carry_checks),
    )
