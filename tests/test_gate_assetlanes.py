"""Two architecture lanes, built at once, reported together.

A hosted release runner has an observed hard lifetime below the workflow's
nominal timeout, so the four-cell profile/architecture matrix only fits if both
architectures build concurrently. That is where the shell version was weakest:
each lane's status came back through `wait` into a variable, and a variable
that goes unread turns a failed build into a passing gate.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate.assetlanes import AssetLanes, Profile, discover_profiles
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
ARCHES = (CONFIG.arch("arm64"), CONFIG.arch("x86_64"))



def _build_all(lanes) -> None:
    """Every architecture's lane, as the plan schedules them.

    `run(architectures)` drove both on a thread pool. They are two steps in
    one wave now -- holding Docker shared, so they still overlap each other
    while excluding everything else -- and a caller that wants both says so.
    """
    for arch in ARCHES:
        lanes.build(arch)


def _checkout(tmp_path: Path, *, profiles: tuple[str, ...] = ("code", "co-work")) -> Path:
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    for name in profiles:
        directory = tmp_path / "config" / "profiles" / name
        directory.mkdir(parents=True)
        (directory / "profile.toml").write_text(f'id = "{name}"\n')
    return tmp_path


class Building(RecordingRunner):
    """A runner whose build lanes produce the artifacts they are asked for."""

    def __init__(self, root: Path, *, omit: str | None = None, **kwargs) -> None:
        super().__init__(root, **kwargs)
        self._omit = omit

    def execute(self, command):
        completed = super().execute(command)
        # Keyed on the argv that actually reaches the builder, not on a
        # dispatcher's. Fabricating artifacts from the recipe's arguments was
        # what let the lane's output root be dropped one layer further down
        # without a single test noticing.
        if "--output" in command.argv:
            output = command.argv[command.argv.index("--output") + 1]
            arch = command.argv[command.argv.index("--arch") + 1]
            if command.log is not None:
                command.log.parent.mkdir(parents=True, exist_ok=True)
                with command.log.open("a", encoding="utf-8") as sink:
                    sink.write(f"building {arch}\n")
            produced = Path(output) / arch
            produced.mkdir(parents=True, exist_ok=True)
            config = gate_config.for_root(self.root)
            for name in (*config.artifacts.bootable, *config.assets.evidence_artifacts):
                if name == self._omit:
                    continue
                (produced / name).write_text("bytes")
        return completed


def _lanes(runner: RecordingRunner, root: Path) -> AssetLanes:
    config = gate_config.for_root(root)
    return AssetLanes(runner, config, discover_profiles(config))


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------


def test_profiles_come_from_the_glob_the_config_declares(tmp_path: Path) -> None:
    root = _checkout(tmp_path, profiles=("code", "co-work"))

    found = discover_profiles(gate_config.for_root(root))

    assert [profile.name for profile in found] == ["co-work", "code"]


def test_a_checkout_with_no_profiles_says_which_pattern_matched_nothing(
    tmp_path: Path,
) -> None:
    root = _checkout(tmp_path, profiles=())

    with pytest.raises(GateError, match="no profiles matched"):
        discover_profiles(gate_config.for_root(root))


# ---------------------------------------------------------------------------
# Building
# ---------------------------------------------------------------------------


def test_every_profile_is_built_for_every_architecture(tmp_path: Path) -> None:
    root = _checkout(tmp_path)
    runner = Building(root)

    _build_all(_lanes(runner, root))

    # Matched on the builder invocation rather than on a recipe name: the
    # dispatcher is gone, and matching it would have kept passing while the
    # build reached the wrong directory.
    for arch in ARCHES:
        for profile in ("code", "co-work"):
            for stage in CONFIG.imagebuild.lane_templates:
                assert runner.matching(
                    rf"--profile \S*{profile}\S* .*--template {stage}.*--arch {arch.name}"
                ), f"{profile}/{arch.name}/{stage} was never built"


def test_each_lane_writes_to_its_own_output_root(tmp_path: Path) -> None:
    """One shared root is how two concurrent lanes race over `current`."""
    root = _checkout(tmp_path)
    runner = Building(root)

    lanes = _lanes(runner, root)
    _build_all(lanes)

    profile = Profile(name="code", manifest=root / "config/profiles/code/profile.toml")
    outputs = {lanes.lane_assets(profile, arch) for arch in ARCHES}
    assert len(outputs) == len(ARCHES)


def test_each_lane_writes_to_its_own_log(tmp_path: Path) -> None:
    """Two lanes streaming to one terminal interleave into unreadable output."""
    root = _checkout(tmp_path)
    runner = Building(root)

    _build_all(_lanes(runner, root))

    logs = {command.log for command in runner.commands if command.log is not None}
    assert len(logs) == len(ARCHES)


def test_a_lane_producing_nothing_fails_rather_than_carrying_on(
    tmp_path: Path,
) -> None:
    """A build that exits zero having written no kernel fails here, by name,
    instead of much later inside a VM boot."""
    root = _checkout(tmp_path)
    runner = Building(root, omit="vmlinuz")

    # The lane raises for itself now; aggregating both is the scheduler's job,
    # which `test_both_lanes_are_awaited_even_when_the_first_fails` covers.
    with pytest.raises(GateError, match="did not produce non-empty"):
        _build_all(_lanes(runner, root))


def test_both_lanes_are_awaited_even_when_the_first_fails(tmp_path: Path) -> None:
    """Cancelling the second would leave its containers running, and would
    report one error for a run that had two.

    The pool used to guarantee this by awaiting every future itself. It is the
    scheduler's rule now, and a stronger one: two steps with no edge between
    them both run, and a failure skips only what *depends* on it. Asserted
    through a plan, because that is where the guarantee lives.
    """
    from capsem.gate.actions import Call
    from capsem.gate.context import Context
    from capsem.gate.execution import step
    from capsem.gate.opacity import CallJustification, OpaqueKind
    from capsem.gate.plan import Plan

    root = _checkout(tmp_path)
    runner = Building(root, omit="vmlinuz")
    lanes = _lanes(runner, root)

    plan = Plan("lanes")
    for arch in ARCHES:
        plan.add(
            step(
                f"build.{arch.name}",
                Call(arch.name, lambda _ctx, a=arch: lanes.build(a), justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="a synthetic step whose work is decided by the test",
                    effects=frozenset(),
                )),
                contends=(CONFIG.shared("docker_daemon"),),
            )
        )

    with pytest.raises(GateError) as failure:
        plan.run(Context(runner, CONFIG))

    for arch in ARCHES:
        assert f"build.{arch.name}" in str(failure.value), (
            "a lane that failed beside another must still be named"
        )


def test_a_failing_lane_surfaces_the_tail_of_its_log(tmp_path: Path) -> None:
    root = _checkout(tmp_path)
    runner = Building(root, omit="rootfs.erofs")

    with pytest.raises(GateError):
        _build_all(_lanes(runner, root))

    assert any("--- tail of" in note for note in runner.notes)
    assert any("asset-build lane failed" in note for note in runner.notes)


def test_a_lane_whose_log_is_missing_says_so_rather_than_crashing(
    tmp_path: Path,
) -> None:
    """The report runs on the failure path; a second failure there would
    replace the error the operator needs to read."""
    root = _checkout(tmp_path)

    class NeverLogs(Building):
        """A lane that fails before writing anything to its log."""

        def execute(self, command):
            from dataclasses import replace

            return super().execute(replace(command, log=None))

    runner = NeverLogs(root, omit="initrd.img")

    with pytest.raises(GateError):
        _build_all(_lanes(runner, root))

    assert any("expected lane log is missing" in note for note in runner.notes)


# ---------------------------------------------------------------------------
# The output root, all the way to the builder
# ---------------------------------------------------------------------------


def test_each_lane_tells_the_builder_where_to_write(tmp_path: Path) -> None:
    """The lane's isolation has to survive the whole way down.

    `_build-image-template` declared an `output` parameter and never forwarded
    it, so `capsem-admin` wrote to the one configured assets directory while
    each lane checked a per-lane directory nothing had written. Two concurrent
    architectures then overwrote each other in the shared tree.

    Asserted on the argv that actually reaches the builder, not on the argv the
    lane hands to a dispatcher: the defect lived precisely in the layer between
    those two, which is why every existing test walked straight past it.
    """
    root = _checkout(tmp_path, profiles=("code",))
    runner = Building(root)
    lanes = _lanes(runner, root)
    (profile,) = discover_profiles(gate_config.for_root(root))

    _build_all(lanes)

    for arch in ARCHES:
        expected = str(lanes.lane_assets(profile, arch))
        issued = [c for c in runner.commands if "--output" in c.argv]
        assert any(
            c.argv[c.argv.index("--output") + 1] == expected for c in issued
        ), (
            f"the {arch.name} lane did not tell the builder to write to "
            f"{expected}; it issued:\n  " + "\n  ".join(str(c) for c in issued)
        )


def test_two_lanes_never_name_the_same_output_root(tmp_path: Path) -> None:
    """The property the isolation exists for, stated directly."""
    root = _checkout(tmp_path, profiles=("code",))
    runner = Building(root)
    lanes = _lanes(runner, root)

    _build_all(lanes)

    outputs = {
        command.argv[command.argv.index("--output") + 1]
        for command in runner.commands
        if "--output" in command.argv
    }

    assert len(outputs) == len(ARCHES), f"lanes shared an output root: {outputs}"
