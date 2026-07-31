from __future__ import annotations

import subprocess
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CHECKER = PROJECT_ROOT / "scripts" / "check-source-syntax.py"


def _run(*paths: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(CHECKER), *(str(path) for path in paths)],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


def test_source_syntax_checker_accepts_repository_sources() -> None:
    result = _run()

    assert result.returncode == 0, result.stdout + result.stderr
    assert "YAML" in result.stdout
    assert "Python" in result.stdout
    assert "shell" in result.stdout
    assert "JSON" in result.stdout
    assert "TOML" in result.stdout


def test_source_syntax_checker_rejects_malformed_yaml(tmp_path: Path) -> None:
    workflow = tmp_path / "broken.yaml"
    workflow.write_text("jobs:\n  test: [\n", encoding="utf-8")

    result = _run(workflow)

    assert result.returncode != 0
    assert str(workflow) in result.stderr


def test_source_syntax_checker_rejects_duplicate_yaml_keys(tmp_path: Path) -> None:
    workflow = tmp_path / "duplicate.yaml"
    workflow.write_text(
        "jobs:\n  test:\n    runs-on: ubuntu-latest\n"
        "    runs-on: macos-latest\n",
        encoding="utf-8",
    )

    result = _run(workflow)

    assert result.returncode != 0
    assert "duplicate key" in result.stderr


def test_source_syntax_checker_rejects_malformed_shell(tmp_path: Path) -> None:
    script = tmp_path / "broken.sh"
    script.write_text("#!/bin/bash\nif true; then\n", encoding="utf-8")

    result = _run(script)

    assert result.returncode != 0
    assert str(script) in result.stderr


def test_source_syntax_checker_rejects_malformed_python(tmp_path: Path) -> None:
    script = tmp_path / "broken.py"
    script.write_text("def broken(:\n", encoding="utf-8")

    result = _run(script)

    assert result.returncode != 0
    assert str(script) in result.stderr
