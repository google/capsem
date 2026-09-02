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
import json
from pathlib import Path

import blake3
import pytest
from capsem_builder.gate import cli  # noqa: F401 - imported so every command registers
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.command import GateCommand
from capsem_builder.gate.context import Context
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.sourcecommit import SourceCommit
from helpers.gate import RecordingJournal, RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)


def _rooted(config, root: Path):
    """The same configuration, reading a different checkout."""
    return config.model_copy(update={"root": root})


def _seed_completed_assets(config, root: Path) -> Path:
    arch = config.host_arch()
    tree = root / config.imagebuild.output / arch.name
    tree.mkdir(parents=True)
    entries = {}
    for name in (*config.artifacts.bootable, *config.assets.evidence_artifacts):
        payload = (
            json.dumps(
                {
                    "bomFormat": "CycloneDX",
                    "metadata": {
                        "tools": {"components": [{"name": "cdxgen", "version": "12.7.0"}]},
                        "component": {
                            "type": "operating-system",
                            "name": f"capsem-rootfs-{arch.name}",
                            "version": "guest-rootfs",
                            "properties": [
                                {
                                    "name": "capsem:evidence:scope",
                                    "value": "exported-rootfs",
                                },
                                {"name": "capsem:guest:architecture", "value": arch.name},
                            ],
                        },
                    },
                    "components": [{"purl": "pkg:deb/debian/base-files@1"}],
                }
            ).encode()
            if name == config.assets.obom_artifact
            else b"bytes"
        )
        (tree / name).write_bytes(payload)
        entries[name] = {
            "hash": blake3.blake3(payload).hexdigest(),
            "sha256": "0" * 64,
            "size": len(payload),
        }
    manifest = root / config.imagebuild.output / config.install.manifest_name
    manifest.write_text(
        json.dumps(
            {
                "format": 2,
                "assets": {
                    "current": "test",
                    "releases": {"test": {"arches": {arch.name: entries}}},
                },
            }
        ),
        encoding="utf-8",
    )
    return tree


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
    from capsem_builder.gate import imagebuild

    config = gate_config.load(PROJECT_ROOT)
    arch = config.host_arch()
    tree = _seed_completed_assets(config, tmp_path)
    for name in config.artifacts.bootable:
        (tree / name).write_bytes(b"")

    assert imagebuild.missing(_rooted(config, tmp_path), arch) == list(config.artifacts.bootable)


def test_a_manifest_bound_complete_asset_cohort_counts(tmp_path: Path) -> None:
    """The other half, so the completion check cannot pass by always failing."""
    from capsem_builder.gate import imagebuild

    config = gate_config.load(PROJECT_ROOT)
    arch = config.host_arch()
    _seed_completed_assets(config, tmp_path)

    assert imagebuild.missing(_rooted(config, tmp_path), arch) == []


# ---------------------------------------------------------------------------
# A symlink that points somewhere else with the same name
# ---------------------------------------------------------------------------


def test_a_symlink_is_verified_by_where_it_actually_points(
    tmp_path: Path, context: Context
) -> None:
    """`Symlink` compared basenames, so `a/current` and `b/current` agreed.

    `cache/target/assets/current` decides which architecture the host VM proof boots
    against. Two lanes both leave something called `current`, and checking the
    name proves only that *a* link exists.
    """
    from capsem_builder.gate.fileactions import Symlink

    wanted = tmp_path / "arm64"
    decoy = tmp_path / "nested" / "arm64"
    wanted.mkdir()
    decoy.mkdir(parents=True)
    link = tmp_path / "current"
    link.symlink_to(decoy)

    Symlink(link, str(wanted)).perform(context)

    assert link.resolve() == wanted.resolve()


def test_a_symlink_refuses_to_replace_a_real_directory(tmp_path: Path, context: Context) -> None:
    """Replacing a directory with a link would delete whatever is in it."""
    from capsem_builder.gate.fileactions import Symlink

    target = tmp_path / "arm64"
    target.mkdir()
    occupied = tmp_path / "current"
    occupied.mkdir()
    (occupied / "keep").write_text("bytes")

    with pytest.raises(GateError):
        Symlink(occupied, str(target)).perform(context)

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
                dry_run=False,
                graph=False,
                timing=False,
                channel="prod",
                profile="code",
                source_commit=SourceCommit("0" * 40),
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
                source_commit=SourceCommit("0" * 40),
            ),
        )._describe()


def test_a_known_channel_and_profile_are_accepted() -> None:
    """So the two refusals above cannot pass by refusing everything."""
    plan = GateCommand.registry["release-profile"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(
            dry_run=False,
            graph=False,
            timing=False,
            channel="nightly",
            profile="code",
            source_commit=SourceCommit("0" * 40),
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
        if not command.exclusive and command.__module__.startswith("capsem_builder.gate.")
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
        # It runs the source-contract suite, which contains the gate's own
        # tests. Holding the machine lock while running tests *about* the gate
        # stalls the command outright -- 27 minutes against 4:41 direct.
        "test-release-contracts",
        "version",
    }, f"a command changed its exclusivity: {sorted(read_only)}"


def test_the_commands_that_write_shared_artifacts_are_exclusive() -> None:
    """Named individually, because each was non-exclusive and each mutates.

    Per-step `[execution.exclusives]` are `threading.Lock`s: they order steps
    inside one plan and coordinate nothing between two `capsem-gate` processes.
    So `just _sign` in one terminal could replace the binaries a qualification
    in another was running.
    """
    for name in ("sign", "build-ui", "install-tools", "install-node"):
        assert GateCommand.registry[name].exclusive, (
            f"{name} writes shared artifacts without holding the machine lock"
        )


# ---------------------------------------------------------------------------
# Building a plan cannot cost anything
# ---------------------------------------------------------------------------


def test_no_command_touches_the_machine_while_building_its_plan() -> None:
    """Every registered command, not just the one that was caught.

    `release.py` built its own `Runner` inside `plan()` to capture `git
    rev-parse HEAD`, so `--dry-run` ran a command and the answer could go stale
    between being printed and being executed. The seal catches it at runtime;
    this catches it for every command at once, without a machine.
    """
    from capsem_builder.gate.errors import GateError

    arguments = {
        "exec": {"guest_command": "true"},
        "release-binaries": {
            "channel": "nightly",
            "source_commit": SourceCommit("0" * 40),
        },
        "release-profile": {
            "channel": "nightly",
            "profile": "code",
            "source_commit": SourceCommit("0" * 40),
        },
        "cross-compile": {"arch": "arm64"},
        "prove-deb": {
            "package": "x.deb",
            "content_root": "cache/target/content",
            "manifest_url": "file:///m",
            "channel": "nightly",
        },
        "build-assets": {"profile": "code", "arch": "arm64", "template": "all"},
        "dev": {"surface": "ui", "args": []},
        "logs": {"target": ""},
        "runs": {"action": "list", "run": None, "failed": False, "other": None},
    }

    for name, command in sorted(GateCommand.registry.items()):
        if not command.__module__.startswith("capsem_builder.gate."):
            continue
        runner = RecordingRunner(PROJECT_ROOT)
        args = argparse.Namespace(
            dry_run=False, graph=False, timing=False, **arguments.get(name, {})
        )
        try:
            command(runner, args)._describe()
        except GateError as failure:
            assert "plan() must describe work" not in str(failure), (
                f"{name} touches the machine while building its plan"
            )
        except (AttributeError, TypeError):
            continue  # this command needs arguments this test does not model
        assert not runner.commands, f"{name} issued a command while planning"


# ---------------------------------------------------------------------------
# Freshness that cannot be wrong
# ---------------------------------------------------------------------------


def test_guest_binary_identity_covers_more_than_rust_sources() -> None:
    """The content key includes workspace inputs that affect guest bytes."""
    from capsem_builder.gate import config as gate_config
    from capsem_builder.image.config import load_guest_config

    config = gate_config.load(PROJECT_ROOT)
    build = load_guest_config(config.path(config.imagebuild.source_config)).build
    watched = set(build.guest_rust_builder.source_roots)

    assert "Cargo.lock" in watched
    assert "Cargo.toml" in watched
    assert config.package.toolchain_pin in watched
