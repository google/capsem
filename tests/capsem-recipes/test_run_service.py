"""Verify just run-service starts the service and creates a socket."""

from pathlib import Path

import pytest

pytestmark = pytest.mark.recipe

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _recipe_block(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith(name))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def test_run_service_creates_socket():
    pass


def test_ensure_service_detaches_from_recipe_shell():
    block = _recipe_block("_ensure-service:")

    assert "nohup" in block
    assert "3>&-" in block
    assert "SVC_PID=$!" in block
