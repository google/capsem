"""Building the VM runtime assets, both architectures at once.

A hosted release runner has an observed hard lifetime below the workflow's
nominal timeout, so the build only fits if the two architectures build
concurrently. Each lane owns a distinct Docker tag
(`capsem-*-<arch>`) and an isolated output root, which is what keeps them from
colliding over tags or over the `current` symlink.

The artifact list, the log tail length, and the scratch root are `[assets]` in
`config/gate.toml`.

Concurrency is where the shell version was weakest. Each lane's output went
through `tee` to its own log, and a failing lane printed a 200-line tail --
useful, but the lane's exit status arrived through `wait` into a variable, and
getting that wrong silently turns a failed build into a passing gate. Here a
lane either returns its log or raises, and both lanes are always awaited before
either result is read.
"""

from __future__ import annotations

from pathlib import Path

from . import assetidentity, assetreceipt, assetstore, imagebuild
from . import config as gate_config
from .actions import Action
from .cachecontrol import CacheControl
from .config import Arch
from .context import Context
from .errors import GateError
from .filesystem import make_dir, remove
from .proc import Runner


def lane_assets(config: gate_config.GateConfig, arch: Arch) -> Path:
    """The isolated output root shared by planning and lane execution."""
    return assetstore.local_lane(config, arch=arch)


def prepare_workspace(config: gate_config.GateConfig) -> None:
    """Remove derived/obsolete output while retaining isolated lane caches."""
    root = config.path(config.assets.test_root)
    if root.is_symlink() or (root.exists() and not root.is_dir()):
        remove(root)
    make_dir(root)
    retained = {lane_assets(config, arch).name for arch in config.architectures.values()}
    for child in tuple(root.iterdir()):
        if child.name not in retained or (not child.is_symlink() and not child.is_dir()):
            remove(child)
    assetstore.materialize(config, assetidentity.lane_identity(config))


class RequireLaneReceipts(Action, name="require-asset-lane-receipts"):
    """Carry only lane outputs whose exact source and bytes still validate."""

    def __init__(
        self,
        config: gate_config.GateConfig,
        arches: tuple[Arch, ...],
        *,
        stages: frozenset[str] = assetreceipt.REUSABLE_STAGES,
    ) -> None:
        self._config = config
        self._arches = arches
        self._stages = stages

    def render(self) -> str:
        return "verify exact source-bound asset lane receipts"

    def perform(self, context: Context) -> None:
        identity = assetidentity.lane_identity(self._config)
        invalid = [
            arch.name
            for arch in self._arches
            if not assetreceipt.validates(
                self._config,
                lane_assets(self._config, arch),
                identity,
                arch=arch,
                stages=self._stages,
                touch=True,
            )
        ]
        if invalid:
            raise GateError("cannot carry invalid asset lane receipts: " + ", ".join(invalid))


class SealPackedReceipts(Action, name="seal-packed-asset-lane-receipts"):
    """The terminal action of initrd packing, before the step may record OK."""

    def __init__(self, config: gate_config.GateConfig) -> None:
        self._config = config

    def render(self) -> str:
        return "record exact packed asset lane receipts"

    def perform(self, context: Context) -> None:
        identity = assetidentity.lane_identity(self._config)
        for arch in self._config.architectures.values():
            assetreceipt.record(
                self._config,
                lane_assets(self._config, arch),
                identity,
                arch=arch,
                stage=assetreceipt.PACKED_STAGE,
            )
        CacheControl(context.runner).enforce("assets", "packed VM assets")


class AssetLanes:
    """One build lane per architecture, run concurrently and reported together."""

    def __init__(self, runner: Runner, config: gate_config.GateConfig) -> None:
        self._runner = runner
        self._config = config
        self._root = config.path(config.assets.test_root)

    def lane_assets(self, arch: Arch) -> Path:
        return lane_assets(self._config, arch)

    def _build(self, arch: Arch) -> None:
        log = self._root / f"build-{arch.name}.log"
        identity = assetidentity.lane_identity(self._config)
        output = self.lane_assets(arch)
        if assetreceipt.validates(self._config, output, identity, arch=arch, touch=True):
            self._runner.note(
                f"Ironbank asset lane {arch.name} is current for {identity}; reusing it"
            )
            self._require_artifacts(output / arch.name)
            return
        assetstore.reset_lane(self._config, output, identity, arch=arch)
        self._runner.step(f"Ironbank asset build lane: {arch.name}")
        for stage in self._config.imagebuild.lane_templates:
            # Straight to the builder, with this lane's output. It used to go
            # through a recipe that accepted an output argument and dropped it
            # -- so every lane wrote into the one shared assets tree while
            # checking a private one.
            self._runner.run(
                imagebuild.build_argv(
                    self._config, arch=arch.name, template=stage, output=str(output)
                ),
                log=log,
            )
        self._require_artifacts(output / arch.name)
        # This action is inside the lane step, so a journal may carry the
        # output only after the source-bound byte receipt exists. Packing
        # overwrites it with the terminal `packed` receipt later.
        assetreceipt.record(
            self._config, output, identity, arch=arch, stage=assetreceipt.BUILD_STAGE
        )

    def _require_artifacts(self, produced: Path) -> None:
        missing = [
            name
            for name in (*self._config.artifacts.bootable, *self._config.assets.evidence_artifacts)
            if not (produced / name).is_file() or (produced / name).stat().st_size == 0
        ]
        if missing:
            raise GateError(
                "asset build did not produce non-empty "
                + ", ".join(str(produced / name) for name in missing)
            )

    def build(self, arch: Arch) -> None:
        """One architecture's lane, as a step the plan schedules.

        This was `run(architectures)` driving a `ThreadPoolExecutor`: two lanes
        overlapping because they must to fit the time budget, and a graph that
        could not see either of them. It could not order anything against a
        lane, time one, or attribute a failure to one -- the pool reported
        both failures by hand because nothing else could.

        The lanes are steps now, holding Docker *shared* so they still overlap
        each other while excluding every other Docker step. Both still run even
        when one fails, because the scheduler skips only what depends on a
        failed step and these depend on each other not at all.
        """
        make_dir(self._root)
        try:
            self._build(arch)
        except BaseException as error:
            self._report(arch, error)
            raise

    def _report(self, arch: Arch, error: BaseException) -> None:
        log = self._root / f"build-{arch.name}.log"
        self._runner.note(f"ERROR: Ironbank {arch.name} asset-build lane failed: {error}")
        if not log.is_file():
            self._runner.note(f"ERROR: expected lane log is missing: {log}")
            return
        tail = log.read_text(encoding="utf-8", errors="replace").splitlines()
        self._runner.note(f"--- tail of {log} ---")
        for line in tail[-self._config.assets.failure_tail_lines :]:
            self._runner.note(line)
        self._runner.note(f"--- complete log: {log} ---")
