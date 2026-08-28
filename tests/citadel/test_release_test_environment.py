"""Citadel guard for release control-plane isolation in nested tests."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROBE = PROJECT_ROOT / "tests/fixtures/release_source_environment_probe.py"


def test_pytest_strips_the_parent_release_source_from_every_test() -> None:
    variable = gate_config.load(PROJECT_ROOT).environment.source_commit
    environment = {**os.environ, variable: "1" * 40}

    result = subprocess.run(
        [sys.executable, "-m", "pytest", "-q", str(PROBE)],
        cwd=PROJECT_ROOT,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stdout + result.stderr
