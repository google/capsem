"""Refuse a Bash command that would run repository Python on the system interpreter.

Claude Code runs this before every Bash tool call with the call as JSON on
stdin. Exit 2 blocks the command and shows stderr to the agent; exit 0 lets it
run. On macOS `python3` is Apple's 3.9, and repository code run on it fails as
something else (a missing tomllib, a phantom syntax error), so the agent is
pointed at the project interpreter instead.

This file itself runs under whatever `python3` is, so it stays stdlib-only and
3.9-compatible; tests/citadel/test_agent_python_interpreter.py holds both.
"""

import ast
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
USE_INSTEAD = "uv run --project build_system --frozen python"
#: A launcher containing this moves itself onto the project interpreter.
REEXEC = "reexec_project_python"

# A command word: line start or a shell separator, then optional wrappers and
# VAR=value assignments, then a system python.
COMMAND_START = r"(?:^|[;&|(\n]|\$\()"
PREFIX = r"(?:\s*(?:env|time|exec|nohup|command)\b|\s*[A-Za-z_][A-Za-z0-9_]*=\S*)*"
PYTHON = r"\s*((?:/usr/bin/|/usr/local/bin/)?python(?:3(?:\.\d+)?)?)(?=\s|$|\))"
BARE = re.compile(COMMAND_START + PREFIX + PYTHON + r"([^\n;&|)]*)")


def _runs_a_reexecing_launcher(arguments):
    words = arguments.split()
    if not words:
        return False
    script = (ROOT / words[0]).resolve()
    try:
        tree = ast.parse(script.read_text(encoding="utf-8"))
    except (OSError, SyntaxError, ValueError):
        return False
    # A call, not a mention: importing the name re-execs nothing.
    return any(
        isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == REEXEC
        for node in ast.walk(tree)
    )


def offending(command):
    """The first bare system-python invocation in `command`, or None."""
    for match in BARE.finditer(command):
        if not _runs_a_reexecing_launcher(match.group(2)):
            return match.group(0).strip(" ;&|(\n$")
    return None


def main():
    try:
        call = json.load(sys.stdin)
    except ValueError:
        return 0  # Not ours to judge; never break the tool for a bad payload.
    if call.get("tool_name") != "Bash":
        return 0
    command = (call.get("tool_input") or {}).get("command") or ""
    found = offending(command)
    if found is None:
        return 0
    sys.stderr.write(
        f"Refused: `{found}` runs the system Python (Apple's 3.9 on macOS), not the "
        f"project interpreter.\nUse `{USE_INSTEAD}` instead (or `uv run --frozen` inside "
        "sdk/python). Launchers that re-exec themselves, like "
        "build_system/scripts/ci/run-bounded-command.py, are allowed.\n"
    )
    return 2


if __name__ == "__main__":
    sys.exit(main())
