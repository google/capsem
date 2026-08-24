"""The install image is one frozen source product, not two live measurements."""

from __future__ import annotations

import argparse
import importlib
import json
import subprocess
from pathlib import Path

import pytest
from helpers.gate import RECORDED_IMAGE_ID, RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def test_the_observer_knows_the_exact_source_replica_root() -> None:
    assert CONFIG.runlog.source_replica_roots == (CONFIG.candidate.source_snapshot_dir,)


def _source(tmp_path: Path):
    """A tiny real Git subject using the gate's authoritative digest script."""
    subprocess.run(("git", "init", "-q"), cwd=tmp_path, check=True)
    (tmp_path / ".gitignore").write_text("target/\n", encoding="utf-8")
    script = tmp_path / CONFIG.candidate.source_digest_script
    script.parent.mkdir(parents=True)
    script.write_bytes((PROJECT_ROOT / CONFIG.candidate.source_digest_script).read_bytes())
    tracked = tmp_path / "tracked.txt"
    tracked.write_text("recorded source\n", encoding="utf-8")
    subprocess.run(("git", "add", "."), cwd=tmp_path, check=True)
    subprocess.run(
        (
            "git",
            "-c",
            "user.name=Gate Test",
            "-c",
            "user.email=gate@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "subject",
        ),
        cwd=tmp_path,
        check=True,
    )
    return CONFIG.model_copy(update={"root": tmp_path}), tracked


def _capture(config):
    from capsem.gate import snapshot, sourcecapture

    expected = sourcecapture.SourceDigest(snapshot.digest(config.root, config))
    receipt = config.path(config.candidate.source_state_file)
    receipt.parent.mkdir(parents=True, exist_ok=True)
    receipt.write_text(json.dumps({"digest": expected}), encoding="utf-8")
    return sourcecapture.capture(config, expected=expected)


def test_source_record_captures_an_immutable_docker_subject(tmp_path: Path) -> None:
    from capsem.gate import snapshot, sourcecapture

    config, tracked = _source(tmp_path)
    expected = sourcecapture.SourceDigest(snapshot.digest(config.root, config))
    frozen = _capture(config)
    tracked.write_text("transient test mutation\n", encoding="utf-8")

    assert frozen.root == config.path(config.candidate.source_snapshot_dir)
    assert frozen.digest == expected
    assert (frozen.root / "tracked.txt").read_text(encoding="utf-8") == "recorded source\n"
    assert snapshot.digest(frozen.root, config) == expected


def test_build_and_smoke_share_the_persisted_frozen_identity(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem.gate import installbuilder, installimage
    from capsem.gate.installbuilder import InstallBuilderIdentity

    config, tracked = _source(tmp_path)
    frozen = _capture(config)
    runner = RecordingRunner(PROJECT_ROOT)
    helper = InstallBuilderIdentity(
        "capsem-install-builder:test",
        RECORDED_IMAGE_ID,
        "capsem-install-builder:test",
    )
    monkeypatch.setattr(
        installbuilder, "require_local_image", lambda *_args, **_kwargs: helper.input_key
    )
    monkeypatch.setattr(installbuilder, "require_current", lambda *_args, **_kwargs: helper)

    built = installimage.build_source_image(runner, config, identity=helper, source=frozen)
    tracked.write_text("coverage changed the live tree\n", encoding="utf-8")
    selected = installimage.require_local_image(runner, config)

    source_build = next(
        command
        for command in runner.commands
        if command.argv[:2] == ("docker", "build")
        and any(value.endswith("Dockerfile.install-test") for value in command.argv)
    )
    assert source_build.argv[-1] == str(frozen.root)
    assert selected == built.input_key
    assert config.path(config.install.builder.source_identity_file).is_file()


def test_malformed_image_receipt_fails_before_smoke(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem.gate import installbuilder, installimage
    from capsem.gate.installbuilder import InstallBuilderIdentity

    config, _ = _source(tmp_path)
    frozen = _capture(config)
    runner = RecordingRunner(PROJECT_ROOT)
    helper = InstallBuilderIdentity(
        "capsem-install-builder:test",
        RECORDED_IMAGE_ID,
        "capsem-install-builder:test",
    )
    monkeypatch.setattr(
        installbuilder, "require_local_image", lambda *_args, **_kwargs: helper.input_key
    )
    installimage.build_source_image(runner, config, identity=helper, source=frozen)
    receipt = config.path(config.install.builder.source_identity_file)
    payload = json.loads(receipt.read_text(encoding="utf-8"))
    receipt.write_text(json.dumps({**payload, "untrusted": True}), encoding="utf-8")
    before = len(runner.commands)

    with pytest.raises(GateError, match="install image receipt"):
        installimage.require_local_image(runner, config)

    assert len(runner.commands) == before


def test_source_drift_during_build_leaves_no_success_receipt(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem.gate import installbuilder, installimage, sourcecapture
    from capsem.gate.installbuilder import InstallBuilderIdentity

    config, _ = _source(tmp_path)
    frozen = _capture(config)
    runner = RecordingRunner(PROJECT_ROOT)
    helper = InstallBuilderIdentity(
        "capsem-install-builder:test",
        RECORDED_IMAGE_ID,
        "capsem-install-builder:test",
    )
    monkeypatch.setattr(
        installbuilder,
        "require_local_image",
        lambda *_args, **_kwargs: helper.input_key,
    )
    checks = 0

    def require_unchanged(_config, _source) -> None:
        nonlocal checks
        checks += 1
        if checks == 2:
            raise GateError("frozen source snapshot moved")

    monkeypatch.setattr(sourcecapture, "require_snapshot", require_unchanged)
    receipt = config.path(config.install.builder.source_identity_file)
    receipt.parent.mkdir(parents=True, exist_ok=True)
    receipt.write_text("stale", encoding="utf-8")

    with pytest.raises(GateError, match="snapshot moved"):
        installimage.build_source_image(runner, config, identity=helper, source=frozen)

    assert checks == 2
    assert not receipt.exists()


def test_install_image_receipt_is_a_resume_checked_product() -> None:
    from capsem.gate.command import GateCommand

    importlib.import_module("capsem.gate.cli")
    plan = GateCommand.registry["install-image"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )._describe()
    built = plan.step_named("install.image-build")

    assert ("source.record", "install.image-build") in plan.edges
    assert built.produces == (CONFIG.path(CONFIG.install.builder.source_identity_file),)
    assert [check.render() for check in built.carry_checks] == [
        "require the exact receipted install qualification image"
    ]
