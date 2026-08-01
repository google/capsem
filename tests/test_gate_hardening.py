"""The smaller correctness gaps, folded into the primitives that own them.

Each is a place where a check was *nearly* the check it needed to be, and the
gap only shows on the machine that has the unlucky filesystem, the second
concurrent build, or the artifact that exists but is empty.

They are grouped here rather than spread across the modules' own suites because
what they have in common is the shape: a verification that answers a slightly
easier question than the one being asked.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import pytest
from helpers.gate import RecordingJournal, RecordingRunner

from capsem.gate import cli  # noqa: F401 - imported so every command registers
from capsem.gate import config as gate_config
from capsem.gate.command import GateCommand
from capsem.gate.context import Context
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def _rooted(config, root: Path):
    """The same configuration, reading a different checkout."""
    return config.model_copy(update={"root": root})


@pytest.fixture
def context(tmp_path: Path) -> Context:
    return Context(RecordingRunner(tmp_path), CONFIG, journal=RecordingJournal())


# ---------------------------------------------------------------------------
# An artifact that exists is not an artifact that was built
# ---------------------------------------------------------------------------


def test_a_zero_length_required_asset_does_not_count_as_built(tmp_path: Path) -> None:
    """`imagebuild.missing` asked `is_file()`, so a truncated build passed.

    An empty `vmlinuz` is what a build that ran out of disk leaves behind, and
    the check that follows it is the one meant to catch that.
    """
    from capsem.gate import imagebuild

    config = gate_config.load(PROJECT_ROOT)
    arch = config.host_arch()
    tree = tmp_path / config.imagebuild.output / arch.name
    tree.mkdir(parents=True)
    for name in config.imagebuild.required:
        (tree / name).write_bytes(b"")

    assert imagebuild.missing(_rooted(config, tmp_path), arch) == list(
        config.imagebuild.required
    )


def test_a_present_non_empty_asset_counts(tmp_path: Path) -> None:
    """The other half, so the check above cannot pass by always failing."""
    from capsem.gate import imagebuild

    config = gate_config.load(PROJECT_ROOT)
    arch = config.host_arch()
    tree = tmp_path / config.imagebuild.output / arch.name
    tree.mkdir(parents=True)
    for name in config.imagebuild.required:
        (tree / name).write_bytes(b"bytes")

    assert imagebuild.missing(_rooted(config, tmp_path), arch) == []


# ---------------------------------------------------------------------------
# A symlink that points somewhere else with the same name
# ---------------------------------------------------------------------------


def test_a_symlink_is_verified_by_where_it_actually_points(
    tmp_path: Path, context: Context
) -> None:
    """`Symlink` compared basenames, so `a/current` and `b/current` agreed.

    `assets/current` decides which architecture the host VM proof boots
    against. Two lanes both leave something called `current`, and checking the
    name proves only that *a* link exists.
    """
    from capsem.gate.fileactions import Symlink

    wanted = tmp_path / "arm64"
    decoy = tmp_path / "nested" / "arm64"
    wanted.mkdir()
    decoy.mkdir(parents=True)
    link = tmp_path / "current"
    link.symlink_to(decoy)

    Symlink(link, wanted).perform(context)

    assert link.resolve() == wanted.resolve()


def test_a_symlink_refuses_to_replace_a_real_directory(
    tmp_path: Path, context: Context
) -> None:
    """Replacing a directory with a link would delete whatever is in it."""
    from capsem.gate.fileactions import Symlink

    target = tmp_path / "arm64"
    target.mkdir()
    occupied = tmp_path / "current"
    occupied.mkdir()
    (occupied / "keep").write_text("bytes")

    with pytest.raises(GateError):
        Symlink(occupied, target).perform(context)

    assert (occupied / "keep").is_file()


# ---------------------------------------------------------------------------
# Refusing early, on the cheap facts
# ---------------------------------------------------------------------------


def test_a_release_profile_refuses_an_unknown_channel_before_the_gate() -> None:
    """`release-binaries` validated its channel and `release-profile` did not.

    Both spend a complete gate before publishing, so an unknown channel that
    surfaces afterwards costs the whole run to learn something knowable in
    milliseconds.
    """
    with pytest.raises(GateError, match="unknown channel"):
        GateCommand.registry["release-profile"](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(
                dry_run=False, graph=False, timing=False, channel="prod", profile="code"
            ),
        )._describe()


def test_a_release_profile_refuses_an_unknown_profile_before_the_gate() -> None:
    """The same argument for the other argument."""
    with pytest.raises(GateError, match="unknown profile"):
        GateCommand.registry["release-profile"](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(
                dry_run=False,
                graph=False,
                timing=False,
                channel="nightly",
                profile="no-such-profile",
            ),
        )._describe()


def test_a_known_channel_and_profile_are_accepted() -> None:
    """So the two refusals above cannot pass by refusing everything."""
    plan = GateCommand.registry["release-profile"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(
            dry_run=False, graph=False, timing=False, channel="nightly", profile="code"
        ),
    )._describe()

    assert plan.labels


# ---------------------------------------------------------------------------
# Mutation is exclusive unless something proves otherwise
# ---------------------------------------------------------------------------


def test_every_command_that_can_mutate_holds_the_machine_lock() -> None:
    """Per-step exclusives are `threading.Lock`s: they order steps within one
    plan and say nothing about a second `capsem-gate` process.

    So an external build or sign command could replace a binary while a
    qualification was reading it. Anything that writes takes the machine lock;
    the ones that only read say so explicitly.
    """
    read_only = {
        name
        for name, command in GateCommand.registry.items()
        if not command.exclusive and command.__module__.startswith("capsem.gate.")
    }

    # Only these, and each for a stated reason:
    #
    #   doctor, dev-ready  ask questions about the host
    #   lint, version      read source
    #   runs, logs         read what a previous run wrote
    #   dev                an interactive server a developer runs *beside* work
    #
    # Everything else writes something another process could be reading, so it
    # takes the machine lock. `sign` in particular replaces the codesigned
    # binaries a concurrent VM test is executing.
    assert read_only == {
        "doctor",
        "dev",
        "dev-ready",
        "lint",
        "logs",
        "runs",
        "version",
    }, f"a command changed its exclusivity: {sorted(read_only)}"


def test_the_commands_that_write_shared_artifacts_are_exclusive() -> None:
    """Named individually, because each was non-exclusive and each mutates.

    Per-step `[execution.exclusives]` are `threading.Lock`s: they order steps
    inside one plan and coordinate nothing between two `capsem-gate` processes.
    So `just _sign` in one terminal could replace the binaries a qualification
    in another was running.
    """
    for name in (
        "sign",
        "build-ui",
        "install-tools",
        "install-node",
        "test-release-contracts",
    ):
        assert GateCommand.registry[name].exclusive, (
            f"{name} writes shared artifacts without holding the machine lock"
        )
