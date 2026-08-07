"""Repacking the initrd, without corrupting every tree that shares its inode.

This is the recipe `AtomicReplace` was written for. `create_hash_assets.py`
gives the unhashed `initrd.img` a hash-named hardlink sharing one inode, so a
shell `> "$INITRD"` -- truncate and write in place -- mutates that hardlink's
contents too. A VM mid-`VmConfig::build`, reading the old hash-named path,
then sees new bytes that do not match the embedded hash, and reports
`hash mismatch for ...img: expected X, got Y`. A stress run hitting this loses
two cycles per repack.

The cross-compile is conditional for a plain reason: it is slow, and it is only
needed when a staged binary is missing or older than the sources that produce
it. That was a `find -newer` in shell and is a stat comparison here.
"""

from __future__ import annotations

from pathlib import Path

from .actions import Action, Run, Script
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .errors import GateError
from .execution import Step, step
from .fileactions import AtomicReplace, Copy, MakeDir, Remove
from .plan import Plan
from .versions import workspace_version


def _staging(config: GateConfig) -> Path:
    return config.path(config.initrd.staging) / config.host_arch().name


def _initrd_path(config: GateConfig) -> Path:
    """Where the repack writes, whether or not it is there yet."""
    return config.path(config.imagebuild.output) / config.host_arch().name / config.artifacts.initrd


def _initrd(config: GateConfig) -> Path:
    found = _initrd_path(config)
    if not found.is_file():
        raise GateError(f"initrd not found at {found}; run `just doctor fix` first")
    return found


def needs_rebuild(config: GateConfig) -> bool:
    """Whether any staged guest binary is missing or older than its inputs.

    Its *inputs*, not just its `*.rs` files. A dependency bump, a feature
    change or a toolchain bump leaves every source file older than the staged
    binary while the binary is stale -- and a stale guest binary ships into an
    initrd that does not match the source it claims to have been built from.
    """
    settings = config.initrd
    staged = [_staging(config) / name for name in settings.binaries]
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

    def render(self) -> str:
        return "unpack the initrd, refresh its guest payload, and repack it"

    def perform(self, context: Context) -> None:
        config = context.config
        settings = config.initrd
        target = _initrd(config)
        staging = _staging(config)

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
        context.runner.bash(f"gzip -dc {initrd} | cpio -id", cwd=workdir)

    def _stage(self, config: GateConfig, staging: Path, workdir: Path) -> None:
        settings = config.initrd

        init = workdir / "init"
        Remove(init).perform(self._context)
        Copy(config.path(settings.init), init).perform(self._context)
        init.chmod(settings.init_mode)

        # 555 on every guest binary in the *initrd*, reasserted rather than
        # assumed: the builder applies it after a fresh cross-compile, but a
        # cached staging directory may have had its modes changed since.
        #
        # On the copy only. This chmodded the source as well, and three of
        # those sources are tracked files -- `guest/artifacts/capsem-doctor`
        # and friends, recorded 100755 by git. The source digest hashes
        # `S_IMODE`, and git does not track the write bit, so the change was
        # invisible to `git status` and fatal to `source.verify`: a clean
        # checkout records 755, this drops it to 555, and the run ends an hour
        # later claiming the gate changed its own source. It passed on warm
        # machines because the files were already 555 from an earlier run.
        # The copy's mode is set on the next line regardless, so the source
        # chmod never affected the initrd at all.
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
        context.runner.bash(f"find . | cpio -o -H newc | gzip > {scratch}", cwd=workdir)


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
        previous = (
            phase.add(
                step(
                    "guest-agents",
                    Run([*settings.build, "--arch", config.host_arch().name]),
                    contends=(config.exclusive("docker_daemon"),),
                ),
                after=after,
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

    assets = config.path(config.imagebuild.output)
    manifest = phase.add(
        step(
            "manifest",
            Run([*settings.manifest, str(assets), "--version", workspace_version(config.root)]),
        ),
        after=(packed,),
    )
    return phase.add(
        step(
            "hash-aliases",
            # Hash-named hardlinks, so the dev layout matches the installed
            # one and startup resolves locally instead of falling through
            # to a remote fetch.
            Script(settings.hash_assets, str(assets)),
        ),
        after=(manifest,),
    )
