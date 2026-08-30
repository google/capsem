"""The shared initrd repacker, against real gzip+cpio archives."""

from __future__ import annotations

import gzip
import importlib
import os
import re
import shutil
import stat
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import initrd
from capsem_builder.gate.context import Context
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.initrd import repack_step
from capsem_builder.gate.initrdactions import _Repack
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.proc import Runner
from helpers.gate import RecordingJournal, RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
DIRECT_BUILDER_INPUTS = {
    "config/gate.toml",
    "build_system/builder/image/config.py",
    "build_system/builder/image/models.py",
    "build_system/builder/image/cli.py",
    "build_system/builder/image/docker.py",
    "build_system/builder/image/guestbuilder.py",
    "config/docker/image/build.toml",
    "build_system/docker/Dockerfile.guest-rust-builder",
}


def _archive(path: Path) -> bytes:
    source = path.with_suffix(".source")
    source.mkdir(parents=True)
    initial = source / "init"
    initial.write_text("#!/bin/sh\n", encoding="utf-8")
    initial.chmod(0o755)
    entries = b".\n./init\n"
    packed = subprocess.run(
        ("cpio", "-o", "-H", "newc"),
        cwd=source,
        input=entries,
        capture_output=True,
        check=True,
    ).stdout
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(gzip.compress(packed))
    return path.read_bytes()


def _extract(archive: Path, destination: Path) -> None:
    destination.mkdir()
    subprocess.run(
        ("cpio", "-id"),
        cwd=destination,
        input=gzip.decompress(archive.read_bytes()),
        capture_output=True,
        check=True,
    )


def _config(staging: Path):
    initrd = CONFIG.initrd.model_copy(update={"staging": str(staging)})
    return CONFIG.model_copy(update={"initrd": initrd})


def test_guest_diagnostics_have_one_importable_support_module(monkeypatch) -> None:
    """The shipped tests must not depend on pytest's ambiguous conftest name."""
    monkeypatch.syspath_prepend(str(PROJECT_ROOT / "guest" / "artifacts"))
    support = importlib.import_module("diagnostics.diagnostic_support")
    environment = importlib.import_module("diagnostics.test_environment")

    result = support.run("printf capsem-diagnostics")

    assert environment.run is support.run
    assert result.stdout == "capsem-diagnostics"


def test_guest_doctor_installs_diagnostics_as_an_importable_package(
    tmp_path: Path,
) -> None:
    """The installed directory must preserve the diagnostics package context."""
    artifacts = PROJECT_ROOT / "guest" / "artifacts"
    doctor = (artifacts / "capsem-doctor").read_text()
    match = re.search(r'^TESTS_DIR="([^"]+)"$', doctor, re.MULTILINE)

    assert match is not None
    installed = Path(match.group(1))
    assert installed.name.isidentifier()
    assert str(installed) in (artifacts / "capsem-init").read_text()
    assert str(installed) in (
        PROJECT_ROOT / "config" / "docker" / "Dockerfile.rootfs.j2"
    ).read_text()

    staged = tmp_path / installed.name
    shutil.copytree(artifacts / "diagnostics", staged)
    result = subprocess.run(
        (sys.executable, "-m", "pytest", str(staged), "--collect-only", "-q"),
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == pytest.ExitCode.NO_TESTS_COLLECTED
    assert "ImportError" not in result.stderr


@pytest.mark.parametrize("arch", tuple(CONFIG.architectures))
def test_repack_accepts_an_exact_arch_and_target_without_breaking_hardlinks(
    tmp_path: Path, arch: str
) -> None:
    staging = tmp_path / "staging"
    for name in CONFIG.initrd.binaries:
        binary = staging / arch / name
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(f"{arch}:{name}".encode())

    target = tmp_path / "private assets" / arch / CONFIG.artifacts.initrd
    original = _archive(target)
    alias = tmp_path / "old-by-hash.img"
    os.link(target, alias)
    context = Context(Runner(PROJECT_ROOT), _config(staging), journal=RecordingJournal())

    _Repack(target=target, arch=arch).perform(context)

    assert alias.read_bytes() == original
    extracted = tmp_path / "extracted"
    _extract(target, extracted)
    for name in CONFIG.initrd.binaries:
        payload = extracted / name
        assert payload.read_bytes() == f"{arch}:{name}".encode()
        assert stat.S_IMODE(payload.stat().st_mode) == CONFIG.initrd.binary_mode
    assert stat.S_IMODE((extracted / "init").stat().st_mode) == CONFIG.initrd.init_mode
    for relative in CONFIG.initrd.files:
        assert (extracted / Path(relative).name).is_file()
    for relative in CONFIG.initrd.trees:
        tree = extracted / Path(relative).name
        assert tree.is_dir()
        assert not list(tree.rglob(CONFIG.initrd.prune))


def test_repack_refuses_a_missing_binary_before_replacing_the_target(tmp_path: Path) -> None:
    staging = tmp_path / "staging"
    arch = next(iter(CONFIG.architectures))
    for name in CONFIG.initrd.binaries[1:]:
        binary = staging / arch / name
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"agent")
    target = tmp_path / CONFIG.artifacts.initrd
    original = _archive(target)
    context = Context(Runner(PROJECT_ROOT), _config(staging), journal=RecordingJournal())

    with pytest.raises(GateError, match=CONFIG.initrd.binaries[0]):
        _Repack(target=target, arch=arch).perform(context)

    assert target.read_bytes() == original


def test_matrix_step_builds_only_stale_architecture_staging(tmp_path: Path) -> None:
    arch = next(iter(CONFIG.architectures))
    config = _config(tmp_path / "staging")
    target = tmp_path / CONFIG.artifacts.initrd
    stage = repack_step(config, {arch: (target,)}).actions[0]
    runner = RecordingRunner(PROJECT_ROOT)
    context = Context(runner, config, journal=RecordingJournal())

    stage.perform(context)

    assert runner.rendered == [
        " ".join((*CONFIG.initrd.build, "--arch", arch)),
    ]

    runner.commands.clear()
    for name in CONFIG.initrd.binaries:
        binary = Path(config.initrd.staging) / arch / name
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"fresh")
        os.utime(binary, (4_000_000_000, 4_000_000_000))

    stage.perform(context)

    assert runner.commands == []


def test_standalone_initrd_plan_shape_is_identical_before_and_after_staging(
    tmp_path: Path,
) -> None:
    """Generated output may change actions, never exact-resume graph identity."""
    arch = CONFIG.host_arch().name
    config = _config(tmp_path / "staging")

    cold = Plan("cold-initrd")
    initrd.pack(cold, config)

    for name in config.initrd.binaries:
        binary = Path(config.initrd.staging) / arch / name
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"fresh")
        os.utime(binary, (4_000_000_000, 4_000_000_000))

    warm = Plan("warm-initrd")
    initrd.pack(warm, config)

    assert cold.labels == warm.labels
    assert cold.edges == warm.edges
    assert "initrd.guest-agents" in warm.labels


def test_carried_initrd_staging_is_revalidated_before_resume(tmp_path: Path) -> None:
    arch = CONFIG.host_arch().name
    config = _config(tmp_path / "staging")
    plan = Plan("resume-initrd")
    initrd.pack(plan, config)
    check = plan.step_named("initrd.guest-agents").carry_checks[0]
    context = Context(RecordingRunner(PROJECT_ROOT), config, journal=RecordingJournal())

    with pytest.raises(GateError, match=r"resume from initrd\.guest-agents"):
        check.perform(context)

    for name in config.initrd.binaries:
        binary = Path(config.initrd.staging) / arch / name
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"fresh")
        os.utime(binary, (4_000_000_000, 4_000_000_000))

    check.perform(context)


def test_freshness_inventory_covers_the_direct_builder_inputs() -> None:
    assert set(CONFIG.initrd.freshness_inputs) >= DIRECT_BUILDER_INPUTS


@pytest.mark.parametrize("relative", CONFIG.initrd.freshness_inputs)
def test_touching_each_declared_input_rebuilds_warm_staging(tmp_path: Path, relative: str) -> None:
    config = CONFIG.model_copy(update={"root": tmp_path})
    arch = next(iter(config.architectures))
    for declared in config.initrd.freshness_inputs:
        source = config.path(declared)
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_bytes(b"input")
        os.utime(source, (1, 1))
    for name in config.initrd.binaries:
        binary = config.path(config.initrd.staging) / arch / name
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"warm")
        os.utime(binary, (2, 2))
    os.utime(config.path(relative), (3, 3))
    runner = RecordingRunner(tmp_path)
    context = Context(runner, config, journal=RecordingJournal())
    stage = repack_step(config, {arch: (tmp_path / config.artifacts.initrd,)}).actions[0]

    stage.perform(context)

    assert runner.rendered == [
        " ".join((*config.initrd.build, "--arch", arch)),
    ]
