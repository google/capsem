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

from . import config as gate_config
from . import imagebuild
from .config import Arch
from .errors import GateError
from .filesystem import make_dir
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
        return self._root / profile.name / f"build-{arch.name}"

    def _build(self, arch: Arch) -> None:
        log = self._root / f"build-{arch.name}.log"
        for profile in self._profiles:
            output = self.lane_assets(profile, arch)
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

    def _require_artifacts(self, produced: Path) -> None:
        missing = [
            name
            for name in (
                *self._config.artifacts.bootable,
                *self._config.assets.evidence_artifacts,
            )
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
