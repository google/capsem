"""Building every profile's VM assets, both architectures at once.

A hosted release runner has an observed hard lifetime below the workflow's
nominal timeout, so the four-cell profile/architecture matrix only fits if the
two architectures build concurrently. Each lane owns a distinct Docker tag
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

from dataclasses import dataclass
from pathlib import Path

from . import assetidentity, assetreceipt, imagebuild
from . import config as gate_config
from .actions import Action
from .config import Arch
from .context import Context
from .errors import GateError
from .filesystem import make_dir, remove
from .proc import Runner


@dataclass(frozen=True)
class Profile:
    """A checked-in profile, by the directory that declares it."""

    name: str
    manifest: Path


def discover_profiles(config: gate_config.GateConfig) -> list[Profile]:
    """Every checked-in profile, in the order the gate builds them."""
    pattern = config.assets.profiles_glob
    found = [
        Profile(name=path.parent.name, manifest=path) for path in sorted(config.root.glob(pattern))
    ]
    if not found:
        raise GateError(f"no profiles matched {pattern} under {config.root}")
    return found


def lane_assets(config: gate_config.GateConfig, profile: Profile, arch: Arch) -> Path:
    """The isolated output root shared by planning and lane execution."""
    return config.path(config.assets.test_root) / profile.name / f"build-{arch.name}"


def prepare_workspace(config: gate_config.GateConfig, profiles: list[Profile]) -> None:
    """Remove derived/obsolete output while retaining isolated lane caches."""
    root = config.path(config.assets.test_root)
    if root.is_symlink() or (root.exists() and not root.is_dir()):
        remove(root)
    make_dir(root)
    expected = {profile.name: profile for profile in profiles}
    for child in tuple(root.iterdir()):
        if child.name not in expected or child.is_symlink() or not child.is_dir():
            remove(child)
    for profile in profiles:
        profile_root = root / profile.name
        make_dir(profile_root)
        retained = {
            lane_assets(config, profile, arch).name for arch in config.architectures.values()
        }
        for child in tuple(profile_root.iterdir()):
            if child.name not in retained or child.is_symlink() or not child.is_dir():
                remove(child)


class RequireLaneReceipts(Action, name="require-asset-lane-receipts"):
    """Carry only lane outputs whose exact source and bytes still validate."""

    def __init__(
        self,
        config: gate_config.GateConfig,
        profiles: list[Profile],
        arches: tuple[Arch, ...],
        *,
        stages: frozenset[str] = assetreceipt.REUSABLE_STAGES,
    ) -> None:
        self._config = config
        self._profiles = profiles
        self._arches = arches
        self._stages = stages

    def render(self) -> str:
        return "verify exact source-bound asset lane receipts"

    def perform(self, context: Context) -> None:
        del context
        identity = assetidentity.lane_identity(self._config)
        invalid = [
            f"{profile.name}/{arch.name}"
            for profile in self._profiles
            for arch in self._arches
            if not assetreceipt.validates(
                self._config,
                lane_assets(self._config, profile, arch),
                identity,
                profile=profile.name,
                arch=arch,
                stages=self._stages,
            )
        ]
        if invalid:
            raise GateError("cannot carry invalid asset lane receipts: " + ", ".join(invalid))


class SealPackedReceipts(Action, name="seal-packed-asset-lane-receipts"):
    """The terminal action of initrd packing, before the step may record OK."""

    def __init__(self, config: gate_config.GateConfig, profiles: list[Profile]) -> None:
        self._config = config
        self._profiles = profiles

    def render(self) -> str:
        return "record exact packed asset lane receipts"

    def perform(self, context: Context) -> None:
        del context
        identity = assetidentity.lane_identity(self._config)
        for profile in self._profiles:
            for arch in self._config.architectures.values():
                assetreceipt.record(
                    self._config,
                    lane_assets(self._config, profile, arch),
                    identity,
                    profile=profile.name,
                    arch=arch,
                    stage=assetreceipt.PACKED_STAGE,
                )


class AssetLanes:
    """One build lane per architecture, run concurrently and reported together."""

    def __init__(
        self, runner: Runner, config: gate_config.GateConfig, profiles: list[Profile]
    ) -> None:
        self._runner = runner
        self._config = config
        self._root = config.path(config.assets.test_root)
        self._profiles = profiles

    def lane_assets(self, profile: Profile, arch: Arch) -> Path:
        return lane_assets(self._config, profile, arch)

    def _build(self, arch: Arch) -> None:
        log = self._root / f"build-{arch.name}.log"
        identity = assetidentity.lane_identity(self._config)
        for profile in self._profiles:
            output = self.lane_assets(profile, arch)
            if assetreceipt.validates(
                self._config,
                output,
                identity,
                profile=profile.name,
                arch=arch,
            ):
                self._runner.note(
                    f"Ironbank asset lane {profile.name} ({arch.name}) is current "
                    f"for {identity}; reusing it"
                )
                self._require_artifacts(output / arch.name)
                continue
            remove(output)
            self._runner.step(f"Ironbank asset build lane: {profile.name} ({arch.name})")
            for stage in self._config.imagebuild.lane_templates:
                # Straight to the builder, with this lane's output. It used to
                # go through a recipe that accepted an output argument and
                # dropped it -- so every lane wrote into the one shared assets
                # tree while checking a private one. That recipe is gone: it
                # had no caller left once the lanes came here, and a parameter
                # with no destination is a knob that lies.
                self._runner.run(
                    imagebuild.build_argv(
                        self._config,
                        profile=profile.name,
                        arch=arch.name,
                        template=stage,
                        output=str(output),
                    ),
                    log=log,
                )
            self._require_artifacts(output / arch.name)
            # This action is inside the lane step, so a journal may carry the
            # output only after the source-bound byte receipt exists. Packing
            # overwrites it with the terminal `packed` receipt later.
            assetreceipt.record(
                self._config,
                output,
                identity,
                profile=profile.name,
                arch=arch,
                stage=assetreceipt.BUILD_STAGE,
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
