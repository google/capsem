#!/usr/bin/env python3
"""Print the gate digest for an agent session, and never fail doing it.

Wired as a session-start hook, so it runs before anybody has asked for
anything. That imposes three rules it would not otherwise have.

It must not invoke the gate. `capsem-gate runs digest` is the obvious
implementation and the wrong one: it takes the history lock, it can block
behind a running gate, and a session that hangs on startup is a session nobody
debugs -- they delete the hook. This reads the file the gate already wrote.

It must not need the project's environment. Bare `python3` has no pydantic, so
importing `capsem_builder.gate.config` fails on exactly the machines where a hook is
least welcome to be picky. The only thing needed from config is one path, and
`tomllib` is in the standard library.

And it must not fail. A missing digest is the ordinary state of a fresh
checkout, not an error. It says what is missing and returns success.

`tests/citadel/test_run_digest_echo.py` proves this stays wired to the same key
the gate writes through.
"""

from __future__ import annotations

import json
import os
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(os.environ.get("CAPSEM_REPOSITORY_ROOT", Path.cwd())).resolve()
#: The one setting this needs, named the way the schema names it.
DIGEST_PATH = ("runlog", "digest", "path")

#: How many recent trunk runs to judge health from, and how long to wait.
#:
#: The timeout is what makes this safe in a session-start hook. A hook that
#: hangs is a hook somebody deletes, so an unreachable or slow GitHub costs
#: three seconds and reports that it does not know -- never that things are
#: fine. Everything here degrades to "unknown", which is the one honest answer
#: when the question could not be asked.
CI_RUNS = 20
CI_TIMEOUT_SECONDS = 3.0
CI_WORKFLOW = "CI"
CI_BRANCH = "main"


def trunk_runs() -> list[dict] | None:
    """Recent `CI` conclusions on trunk, newest first, or `None` if unknown.

    Asked here rather than in the gate because the gate cannot ask. Candidate
    and the release lanes run inside the kernel network boundary, so a digest
    written by a run has no way to reach GitHub; the section would be silently
    missing exactly when a run is happening. This hook is on the host, outside
    that boundary, which is the only place the question can be put.
    """
    try:
        finished = subprocess.run(
            [
                "gh",
                "run",
                "list",
                "--workflow",
                CI_WORKFLOW,
                "--branch",
                CI_BRANCH,
                "--limit",
                str(CI_RUNS),
                "--json",
                "status,conclusion,displayTitle,url,createdAt",
            ],
            capture_output=True,
            text=True,
            timeout=CI_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if finished.returncode != 0:
        return None
    try:
        parsed = json.loads(finished.stdout)
    except ValueError:
        return None
    return parsed if isinstance(parsed, list) else None


def consecutive_failures(runs: list[dict]) -> list[dict]:
    """The unbroken run of failures at the head of trunk.

    Cancelled runs are skipped rather than counted or treated as a break: a
    cancellation says nothing about the code, and letting one reset the streak
    is how a wall of red reads as an isolated blip.
    """
    streak = []
    for run in runs:
        if run.get("status") != "completed":
            continue
        conclusion = run.get("conclusion")
        if conclusion == "cancelled":
            continue
        if conclusion != "failure":
            break
        streak.append(run)
    return streak


def ci_section() -> str:
    """What trunk CI is doing, stated before anything else in the session.

    The digest was blind to this and it mattered: trunk sat red for fourteen
    consecutive runs on one root cause while the local digest reported a
    healthy-looking picture, because it only ever described local gate runs.
    An agent read it, saw nothing wrong, and started unrelated work.
    """
    runs = trunk_runs()
    if runs is None:
        return (
            "## Trunk CI\n\n"
            "**Unknown** -- `gh` is missing, unauthenticated, or unreachable. "
            "Not a statement that CI is green.\n"
        )
    if not runs:
        return "## Trunk CI\n\nNo recent `CI` runs on `main`.\n"

    completed = [
        run
        for run in runs
        if run.get("status") == "completed" and run.get("conclusion") != "cancelled"
    ]
    if not completed:
        return "## Trunk CI\n\nNo completed `CI` runs on `main`; health is unknown.\n"

    streak = consecutive_failures(completed)
    if completed[0].get("conclusion") == "success":
        return f"## Trunk CI\n\nGreen on `{CI_BRANCH}`.\n"
    if not streak:
        conclusion = completed[0].get("conclusion", "unknown")
        return f"## Trunk CI\n\nLatest completed run is `{conclusion}`; health is unknown.\n"

    latest = streak[0]
    return (
        "## Trunk CI is RED -- stop the line\n\n"
        f"`{CI_BRANCH}` has failed **{len(streak)} consecutive** `{CI_WORKFLOW}` runs.\n"
        f"Most recent: {latest.get('displayTitle', '?')}\n"
        f"{latest.get('url', '')}\n\n"
        "**Before doing anything else, say one of these out loud:**\n\n"
        "1. what you are changing to fix it, or\n"
        "2. why the work you are about to do is unrelated, and that you are "
        "leaving trunk red on purpose.\n\n"
        "Do not start unrelated work without saying (2). A red trunk that "
        "everyone routes around stops being information: the failure rate is "
        "only a signal while somebody is still reading it, and fourteen runs "
        "of the same error is what it looks like when nobody is.\n"
    )


def digest_path(root: Path) -> Path | None:
    """Where the gate writes the digest, according to config."""
    settings = tomllib.loads((root / "config" / "gate.toml").read_text(encoding="utf-8"))
    for key in DIGEST_PATH:
        if not isinstance(settings, dict) or key not in settings:
            return None
        settings = settings[key]
    return root / settings if isinstance(settings, str) else None


def main() -> int:
    # CI first, and unconditionally. It is the only part of this that can say
    # the tree everyone shares is broken, so it must not sit below a local
    # report that might be long enough to scroll past.
    print(ci_section())
    try:
        target = digest_path(ROOT)
    except (OSError, ValueError) as error:
        print(f"gate digest unavailable ({type(error).__name__}: {error})")
        return 0

    if target is None:
        print(f"gate digest unavailable: config/gate.toml has no {'.'.join(DIGEST_PATH)}")
        return 0
    if not target.is_file():
        print(
            "No gate digest yet. Every `just test` writes one, or run "
            "`uv run --project build_system --frozen capsem-gate runs digest`."
        )
        return 0

    print(target.read_text(encoding="utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
