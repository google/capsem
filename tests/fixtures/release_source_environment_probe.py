"""Child-process probe for pytest's parent-gate environment isolation."""

from __future__ import annotations

import os
from pathlib import Path

from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_parent_release_source_identity_is_not_a_test_identity() -> None:
    variable = gate_config.load(PROJECT_ROOT).environment.source_commit

    assert variable not in os.environ
