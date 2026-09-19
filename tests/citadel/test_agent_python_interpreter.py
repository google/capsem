"""Citadel guard: agents never run repository Python on the system interpreter.

`python3` on a developer Mac is Apple's 3.9. The project runs on the uv-managed
interpreter under `build_system/.venv`. Code that only works on the latter
fails on the former in ways that look like something else: the SessionStart
hook died on `import tomllib` at every session start, so the gate digest the
agent contract says to read first never printed, and a syntax check run with
bare `python3` reported seven "syntax errors" that were only `match`
statements 3.9 cannot parse.

A memory saying "use uv" did not stop it; an agent repeated the mistake the
same day. So a PreToolUse hook refuses the command before it runs, and this
guard keeps the hook registered, precise, and runnable on the interpreter it
exists to catch.
"""

from __future__ import annotations

import ast
import json
import shlex
import subprocess
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SETTINGS = PROJECT_ROOT / ".claude" / "settings.json"
HOOK = PROJECT_ROOT / ".claude" / "hooks" / "refuse-bare-python.py"
#: Marks a launcher that moves itself onto the project interpreter first.
REEXEC = "reexec_project_python"

AGENT_PYTHON_RATIONALE = """\
Repository Python must run on the project interpreter, never the system one.

On macOS `python3` is Apple's 3.9. The builder, tests and SDK need the
uv-managed interpreter, and code run on 3.9 fails as something else entirely
(a missing tomllib, a phantom syntax error). Agents use
`uv run --project build_system --frozen python`; launchers that agents or
hooks invoke as `python3 <script>` must re-exec onto the project interpreter
(bootstrap.reexec_project_python) before importing anything.
"""


def _hooks(event: str) -> list[dict]:
    settings = json.loads(SETTINGS.read_text(encoding="utf-8"))
    return settings.get("hooks", {}).get(event, [])


def _commands(event: str) -> list[str]:
    return [hook["command"] for entry in _hooks(event) for hook in entry.get("hooks", [])]


def _refused(command: str) -> bool:
    """Drive the hook exactly as Claude Code does: tool JSON on stdin."""
    payload = json.dumps({"tool_name": "Bash", "tool_input": {"command": command}})
    result = subprocess.run(
        [sys.executable, str(HOOK)],
        input=payload,
        capture_output=True,
        text=True,
        cwd=PROJECT_ROOT,
        check=False,
        timeout=30,
    )
    assert result.returncode in (0, 2), result
    if result.returncode == 2:
        assert "uv run --project build_system --frozen python" in result.stderr, result.stderr
    return result.returncode == 2


def _calls_reexec(script: Path) -> bool:
    """A call, not a mention: importing the name re-execs nothing."""
    tree = ast.parse(script.read_text(encoding="utf-8"))
    return any(
        isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == REEXEC
        for node in ast.walk(tree)
    )


def test_the_hook_is_registered_for_bash() -> None:
    entries = [entry for entry in _hooks("PreToolUse") if entry.get("matcher") == "Bash"]
    commands = [hook["command"] for entry in entries for hook in entry.get("hooks", [])]
    assert any(HOOK.name in command for command in commands), (
        AGENT_PYTHON_RATIONALE + f"\nno PreToolUse Bash hook runs {HOOK.name}: {commands}"
    )


def test_the_hook_parses_on_the_interpreter_it_catches() -> None:
    """The hook runs under whatever `python3` is, so it must be 3.9 source."""
    ast.parse(HOOK.read_text(encoding="utf-8"), feature_version=(3, 9))


@pytest.mark.parametrize(
    "command",
    [
        "python3 -c 'print(1)'",
        "python -c 'print(1)'",
        "/usr/bin/python3 script.py",
        "python3.9 script.py",
        "cd crates && python3 - <<'PY'\nprint(1)\nPY",
        "echo first\npython3 x.py",
        "true; python3 x.py",
        "false || python3 x.py",
        "echo x | python3 -",
        "FOO=1 BAR=2 python3 x.py",
        "value=$(python3 -c 'print(1)')",
        "(python3 x.py)",
        "env python3 x.py",
        "time python3 x.py",
        # A launcher that does not re-exec is still the system interpreter.
        "python3 build_system/scripts/build/gen_manifest.py",
    ],
)
def test_a_bare_system_python_is_refused(command: str) -> None:
    assert _refused(command), AGENT_PYTHON_RATIONALE + f"\nthe hook let through: {command!r}"


@pytest.mark.parametrize(
    "command",
    [
        "uv run --project build_system --frozen python -c 'print(1)'",
        "uv run --frozen --project sdk/python ty check sdk/python/capsem",
        "build_system/.venv/bin/python x.py",
        "uvx ruff check tests",
        "grep -rn python3 tests/",
        "git log --grep python3",
        # A launcher that re-execs itself onto the project interpreter is safe
        # under any python3, and the agent contract spells this one that way.
        "python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 60 -- cargo test",
    ],
)
def test_a_project_interpreter_passes(command: str) -> None:
    assert not _refused(command), AGENT_PYTHON_RATIONALE + f"\nthe hook refused: {command!r}"


def test_every_python_hook_command_runs_a_self_reexecing_launcher() -> None:
    """A hook's `python3 <script>` runs on 3.9 unless the script re-execs.

    The SessionStart digest printer did not, and failed on `import tomllib`
    at every session start while the guard over it drove `main()` in-process
    on 3.14 and stayed green.
    """
    offenders = []
    for event in ("SessionStart", "PreToolUse", "PostToolUse", "Stop"):
        for command in _commands(event):
            words = shlex.split(command)
            if not words or Path(words[0]).name not in ("python3", "python"):
                continue
            script = PROJECT_ROOT / words[1]
            if script == HOOK:
                continue  # stdlib-only by construction; parsed as 3.9 above
            if not _calls_reexec(script):
                offenders.append(command)
    assert not offenders, AGENT_PYTHON_RATIONALE + f"\nthese hooks run on the system python: {offenders}"
