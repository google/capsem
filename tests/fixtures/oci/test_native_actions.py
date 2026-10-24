"""Existing CLI dispatch keeps finite controls and derived action outputs exact."""

from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.gate.config import load

from tests.fixtures.oci import native_actions, native_handoff

ROOT = Path(__file__).resolve().parents[3]


def args(action, section=None, *, run="12", attempt="3", artifact_id=None):
    return SimpleNamespace(
        action=action, section=section, run_id=run, attempt=attempt, artifact_id=artifact_id
    )


def test_configuration_outputs_derive_all_deadlines_from_the_one_owner(monkeypatch, tmp_path):
    path = tmp_path / "outputs"
    monkeypatch.setenv("GITHUB_OUTPUT", str(path))
    config = load(ROOT)
    native_actions.command(config, args("configure", run=None, attempt=None))
    output = dict(line.split("=", 1) for line in path.read_text().splitlines())
    transfer = config.functional.native_images.transfer
    assert output["outer_seconds"] == str(transfer.outer_seconds)
    assert output["base_outer_seconds"] == str(transfer.base_outer_seconds)
    assert output["settle_seconds"] == str(transfer.verify_seconds + transfer.release_seconds)
    assert output["upload_minutes"] == str(transfer.upload_minutes)
    assert output["download_minutes"] == str(transfer.download_minutes)
    assert Path(output["control_dir"]) == config.path(config.functional.native_images.receipt_dir)


@pytest.mark.parametrize(
    "action,section,run,attempt,artifact_id",
    [
        ("hold", None, "12", "3", None),
        ("ready", "debug_image", None, "3", None),
        ("release", "debug_image", "12", None, "7"),
        ("hold", "debug_image", "12", "3", "7"),
        ("finish", "debug_image", "12", "3", None),
        ("configure", None, "12", "3", None),
    ],
)
def test_cli_rejects_incoherent_owner_controls_before_any_fixture(
    action, section, run, attempt, artifact_id
):
    with pytest.raises(ValueError):
        native_actions.command(
            load(ROOT), args(action, section, run=run, attempt=attempt, artifact_id=artifact_id)
        )


def test_existing_entrypoint_retains_default_select_stage_and_rejects_base_select():
    with pytest.raises(SystemExit):
        native_handoff.main(["select", "base"])
    with pytest.raises(SystemExit):
        native_handoff.main(["stage", "debug_image"])


def test_ready_outputs_only_the_owned_original_upload_and_independent_download(
    monkeypatch, tmp_path
):
    config = load(ROOT)
    output = tmp_path / "outputs"
    monkeypatch.setenv("GITHUB_OUTPUT", str(output))
    owner = SimpleNamespace(key="native-transfer-fixture", section="debug_image")
    monkeypatch.setattr(native_actions.lifetime, "ready", lambda *args: owner)
    monkeypatch.setattr(native_actions.lifetime, "root", lambda *args: tmp_path / "owned")
    native_actions.command(config, args("ready", "debug_image"))
    values = dict(line.split("=", 1) for line in output.read_text().splitlines())
    assert values == {
        "artifact_name": owner.key,
        "artifact_path": str(tmp_path / "owned/upload"),
        "download_path": str(tmp_path / "owned/download"),
    }
