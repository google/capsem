"""Static refusal at the gate's typed correctness boundaries."""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def test_ty_refuses_raw_strings_including_source_commit(tmp_path: Path) -> None:
    """Closed enum and identity seams fail before runtime for typed callers."""
    fixture = PROJECT_ROOT / "tests" / "fixtures" / "typecheck" / "gate_vocabulary_strings.py.txt"
    probe = tmp_path / "gate_vocabulary_strings.py"
    probe.write_bytes(fixture.read_bytes())
    shutil.copytree(
        PROJECT_ROOT / "build_system" / "builder",
        tmp_path / "capsem_builder",
        ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "*.pyo"),
    )

    checked = subprocess.run(
        [
            "uv",
            "run",
            "ty",
            "check",
            "--output-format",
            "concise",
            "--color",
            "never",
            "--python-platform",
            CONFIG.lint.python_platform,
            "--python",
            sys.executable,
            "--extra-search-path",
            str(tmp_path),
            str(probe),
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )

    evidence = checked.stdout + checked.stderr
    assert checked.returncode != 0, evidence
    assert evidence.count("invalid-argument-type") == 9, evidence
    assert 'Expected `Effect`, found `Literal["process"]`' in evidence
    assert 'Expected `InstallImageStep`, found `Literal["install.capacity"]`' in evidence
    assert 'Expected `SandboxMode`, found `Literal["off"]`' in evidence
    assert 'Expected `BuildNetwork`, found `Literal["none"]`' in evidence
    assert 'Expected `ContainerNetwork`, found `Literal["none"]`' in evidence
    assert (
        'Expected `SourceCommit`, found `Literal["0000000000000000000000000000000000000000"]`'
        in evidence
    )
    assert (
        'Expected `SourceSnapshot`, found `Literal["0000000000000000000000000000000000000000000000000000000000000000"]`'
        in evidence
    )
