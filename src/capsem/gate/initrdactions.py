"""Unpacking, restaging and repacking an initrd.

Split from `initrd`, which composes these into steps and was over the
three-hundred-line ceiling. The seam is the one this package uses everywhere:
an `Action` is work, a `Step` is a named unit of it in a plan. These are the
two largest actions in the module by a wide margin.
"""

from __future__ import annotations

from pathlib import Path
from shlex import quote

from .actions import Action, Run
from .config import GateConfig
from .context import Context
from .errors import GateError
from .fileactions import AtomicReplace, Copy, MakeDir, Remove
from .initrdpaths import initrd_at, needs_rebuild, staging_for


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
        target = self._target or initrd_at(config, arch)
        if not target.is_file():
            raise GateError(f"initrd not found at {target}")
        staging = staging_for(config, arch)

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
