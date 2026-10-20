"""The native install plan satisfies the package builder's SBOM contract."""

from __future__ import annotations

import argparse
import importlib
import shlex
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.command import GateCommand
from helpers.gate import RecordingRunner

ROOT = Path(__file__).resolve().parents[3]


def test_local_install_supplies_the_configured_package_sbom(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    importlib.import_module("capsem_builder.gate.cli")
    monkeypatch.setattr("capsem_builder.gate.localinstall.host.on_macos", lambda: True)
    config = gate_config.load(ROOT)
    plan = GateCommand.registry["local-install"](
        RecordingRunner(ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    ).plan()
    command = next(
        line
        for line in plan.describe().splitlines()
        if config.install.local_macos_package_script in line
    )
    argv = shlex.split(command)

    assert "--sbom" in argv, (
        "native installation must supply the package builder's required SBOM output"
    )
    assert argv[argv.index("--sbom") + 1] == str(config.path(config.sbom.output))
