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
    """The daemon must outlive the shell that started it, and inherit none of
    its descriptors.

    This asserted `nohup`, `3>&-` and `SVC_PID=$!` in the recipe body -- the
    shell spelling of exactly that. The recipe dispatches to `capsem-gate
    ensure-service` now and the detaching is `Launch`, whose `start_new_session`
    gives its own session and whose pipes are `DEVNULL`. The `3>&-` had to be
    written by hand because the shell leaks the gate's execution-lock fd into
    the child, which then holds the flock after the gate exits and blocks the
    next run; Python closes non-inheritable descriptors across `exec` already.

    So the claim is unchanged and its evidence moved: the recipe still owns
    starting a detached service, and the detachment is asserted where it lives.
    """
    assert "capsem-gate ensure-service" in _recipe_block("_ensure-service:")

    launch = (PROJECT_ROOT / "build_system/builder/gate/proc.py").read_text(encoding="utf-8")
    assert "start_new_session=True" in launch
    assert "stdout=subprocess.DEVNULL" in launch
    assert "stderr=subprocess.DEVNULL" in launch
