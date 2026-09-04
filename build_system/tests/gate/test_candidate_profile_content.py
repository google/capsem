"""The local candidate packages the real profile bundle IronBank proved."""

from __future__ import annotations

import argparse
from pathlib import Path

from capsem_builder.gate import vmmodules
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.candidate import CandidateCommand
from capsem_builder.gate.content import ProfileContent
from helpers.gate import RecordingRunner

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)


def test_candidate_glowup_consumes_the_real_base_profile(monkeypatch) -> None:
    """The mutable convenience selector is a symlink and not release input."""
    selected: list[ProfileContent] = []
    original = vmmodules.glowup

    def observed(*args, **kwargs):
        selected.append(kwargs["local_content"])
        return original(*args, **kwargs)

    monkeypatch.setattr(vmmodules, "glowup", observed)
    command = CandidateCommand(
        RecordingRunner(ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )
    command._describe()

    expected = ProfileContent.built_profile(CONFIG, CONFIG.suites.pytest.base_profile)
    assert selected == [expected]
    assert selected[0].assets != CONFIG.path(CONFIG.functional.assets_dir)
