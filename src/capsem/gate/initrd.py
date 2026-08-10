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
from shlex import quote

from . import crossexec, host
from .actions import Action, Run, Script
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .errors import GateError
from .execution import Step, step
from .fileactions import AtomicReplace, Copy, MakeDir, Remove
from .imagebases import MaterializeRustBuilders, Prefetch
from .plan import Plan
from .versions import workspace_version


def _staging(config: GateConfig, arch: str | None = None) -> Path:
    selected = config.arch(arch).name if arch else config.host_arch().name
    return config.path(config.initrd.staging) / selected


def _initrd_path(config: GateConfig, arch: str | None = None) -> Path:
    """Where the repack writes, whether or not it is there yet."""
    selected = config.arch(arch).name if arch else config.host_arch().name
    return config.path(config.imagebuild.output) / selected / config.artifacts.initrd


def _initrd(config: GateConfig, arch: str | None = None) -> Path:
    found = _initrd_path(config, arch)
    if not found.is_file():
        raise GateError(f"initrd not found at {found}; run `just doctor fix` first")
    return found


def needs_rebuild(config: GateConfig, arch: str | None = None) -> bool:
    """Whether any staged guest binary is missing or older than its inputs.

    Its *inputs*, not just its `*.rs` files. A dependency bump, a feature
    change or a toolchain bump leaves every source file older than the staged
    binary while the binary is stale -- and a stale guest binary ships into an
    initrd that does not match the source it claims to have been built from.
    """
    settings = config.initrd
    staged = [_staging(config, arch) / name for name in settings.binaries]
    if any(not path.is_file() for path in staged):
        return True

    oldest = min(path.stat().st_mtime for path in staged)
    return any(source.stat().st_mtime > oldest for source in _build_inputs(config))


def _build_inputs(config: GateConfig):
    """Every file whose change should invalidate the staged binaries."""
    settings = config.initrd
    for source_root in settings.sources:
        for pattern in settings.freshness_globs:
            yield from config.path(source_root).rglob(pattern)
    for relative in settings.freshness_inputs:
        candidate = config.path(relative)
        if candidate.is_file():
            yield candidate


class _Repack(Action, name="repack-initrd"):
    """Unpack, replace the guest payload, and repack into a new inode."""

    def __init__(self, *, target: Path | None = None, arch: str | None = None) -> None:
        self._target = target
        self._arch = arch

    def render(self) -> str:
        detail = ""
        if self._target is not None:
            detail = f" {self._target}"
        if self._arch is not None:
            detail += f" from {self._arch} staging"
        return f"unpack the initrd{detail}, refresh its guest payload, and repack it"

    def perform(self, context: Context) -> None:
        config = context.config
        settings = config.initrd
        arch = config.arch(self._arch).name if self._arch else config.host_arch().name
        target = self._target or _initrd(config, arch)
        if not target.is_file():
            raise GateError(f"initrd not found at {target}")
        staging = _staging(config, arch)

        for name in settings.binaries:
            if not (staging / name).is_file():
                raise GateError(f"{name} is missing from {staging}")

        self._context = context

        def build(scratch: Path) -> None:
            workdir = scratch.with_name(scratch.name + ".dir")
            MakeDir(workdir).perform(context)
            try:
                self._unpack(context, target, workdir)
                self._stage(config, staging, workdir)
                self._pack(context, workdir, scratch)
            finally:
                Remove(workdir).perform(context)

        AtomicReplace(target, build).perform(context)

    # -- the pieces --------------------------------------------------------
    def _unpack(self, context: Context, initrd: Path, workdir: Path) -> None:
        context.runner.bash(f"gzip -dc {quote(str(initrd))} | cpio -id", cwd=workdir)

    def _stage(self, config: GateConfig, staging: Path, workdir: Path) -> None:
        settings = config.initrd

        init = workdir / "init"
        Remove(init).perform(self._context)
        Copy(config.path(settings.init), init).perform(self._context)
        init.chmod(settings.init_mode)

        # Set modes on copies only. Changing tracked source modes made a clean
        # gate fail its later byte-and-mode source verification.
        staged = [(staging / name, workdir / name) for name in settings.binaries]
        staged += [
            (config.path(relative), workdir / Path(relative).name) for relative in settings.files
        ]
        for source, target in staged:
            Remove(target).perform(self._context)
            Copy(source, target).perform(self._context)
            target.chmod(settings.binary_mode)

        for relative in settings.trees:
            source = config.path(relative)
            target = workdir / Path(relative).name
            Remove(target).perform(self._context)
            Copy(source, target).perform(self._context)
            for cached in target.rglob(settings.prune):
                Remove(cached).perform(self._context)

    def _pack(self, context: Context, workdir: Path, scratch: Path) -> None:
        command = f"find . | cpio -o -H newc | gzip > {quote(str(scratch))}"
        context.runner.bash(command, cwd=workdir)


class _Stage(Action, name="stage-initrd-agents"):
    """Build one architecture's configured payload only when it is stale."""

    def __init__(self, config: GateConfig, arch: str) -> None:
        self._arch = config.arch(arch).name
        self._run = Run([*config.initrd.build, "--arch", self._arch])

    def render(self) -> str:
        return f"if {self._arch} initrd agents are stale: {self._run.render()}"

    def perform(self, context: Context) -> None:
        if needs_rebuild(context.config, self._arch):
            self._run.perform(context)


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
        ),
        after=after,
    )
    return phase.add(
        step("hash-aliases", Script(config.initrd.hash_assets, str(assets))),
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
                ),
                after=after,
            )
            execution = phase.add(
                step(
                    "guest-execution",
                    crossexec.Require((arch,)),
                    contends=(config.exclusive("docker_daemon"),),
                ),
                after=(base,),
            )
            previous = (
                phase.add(
                    step(
                        "guest-builder",
                        MaterializeRustBuilders((arch,)),
                        contends=(config.exclusive("docker_daemon"),),
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
            produces=(_initrd_path(config),),
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
