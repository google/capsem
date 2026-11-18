"""Only the host's read-only share supplies executable workload controls."""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

SOURCE = Path(__file__).resolve().parents[3] / "guest/artifacts/container/launch.py"


@pytest.fixture
def launcher():
    spec = importlib.util.spec_from_file_location("trusted_launch_inputs", SOURCE)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_options_come_only_from_the_read_only_share(launcher, tmp_path, monkeypatch):
    share = tmp_path / "share"
    share.mkdir()
    writable = tmp_path / "workspace"
    writable.mkdir()
    (share / "options.json").write_text(json.dumps({"env": {"APPROVED": "yes"}}))
    (writable / "options.json").write_text(json.dumps({"env": {"INJECTED": "yes"}}))
    monkeypatch.chdir(writable)
    monkeypatch.setattr(launcher, "IMAGE_SHARE", share)
    assert launcher.launch_options() == {"env": {"APPROVED": "yes"}}
    (share / "options.json").unlink()
    with pytest.raises(FileNotFoundError):
        launcher.launch_options()


def test_launch_options_refuse_links_and_oversize(launcher, tmp_path, monkeypatch):
    monkeypatch.setattr(launcher, "IMAGE_SHARE", tmp_path)
    outside = tmp_path / "other.json"
    outside.write_text("{}")
    options = tmp_path / "options.json"
    options.symlink_to(outside)
    with pytest.raises(OSError):
        launcher.launch_options()
    options.unlink()
    monkeypatch.setattr(launcher, "LAUNCH_CONTROL_LIMIT", 32)
    options.write_bytes(b" " * 33)
    with pytest.raises(ValueError, match="byte limit"):
        launcher.launch_options()


def test_poststart_hook_writes_runtime_markers_outside_the_controller(launcher, tmp_path, monkeypatch):
    workspace = tmp_path / "workspace"
    stage = workspace / launcher.STAGE
    stage.mkdir(parents=True)
    monkeypatch.setattr(launcher, "VM_WORKSPACE", str(workspace))
    monkeypatch.setattr(sys, "argv", [str(SOURCE), "--started"])
    launcher.main()
    assert (stage / "running").read_text() == "1\n"
