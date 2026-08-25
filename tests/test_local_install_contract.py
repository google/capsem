"""The local install path builds one complete product and installs that package."""

from __future__ import annotations

import argparse
import importlib
import tomllib
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate.command import GateCommand

ROOT = Path(__file__).resolve().parents[1]


def _plan(monkeypatch: pytest.MonkeyPatch):
    importlib.import_module("capsem.gate.cli")
    monkeypatch.setattr("capsem.gate.localinstall.host.on_macos", lambda: True)
    return GateCommand.registry["local-install"](
        RecordingRunner(ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    ).plan()


def test_local_install_builds_content_package_then_installs_that_exact_package(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    plan = _plan(monkeypatch)
    labels = list(plan.labels)
    rendered = plan.describe()

    assert labels.index("assets.assemble") < labels.index("local-install.content")
    assert labels.index("local-install.content") < labels.index("local-install.package")
    assert labels.index("local-install.package") < labels.index("local-install.install")
    assert "scripts/build-test-macos-package.sh" in rendered
    assert "sudo /usr/sbin/installer -pkg" in rendered
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))[
        "workspace"
    ]["package"]["version"]
    assert f"packages/Capsem-{version}.pkg" in rendered


def test_public_install_is_only_the_local_install_dispatch() -> None:
    lines = (ROOT / "justfile").read_text(encoding="utf-8").splitlines()
    start = lines.index("install:")
    body = []
    for line in lines[start + 1 :]:
        if line and not line[0].isspace():
            break
        body.append(line)

    assert "capsem-gate local-install" in "\n".join(body)
    assert sum(bool(line.strip()) for line in body) == 1
