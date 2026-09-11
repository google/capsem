"""What a failed pytest step should say first.

The run summary kept the last lines of a failed step's log. A pytest log with
several errors sharing one long assertion message put the actual cause -- a
runc hook's stderr explaining why a real VM refused to start its container --
a hundred lines above that tail, and it had to be grepped out of the console.
Pytest already knows what failed: its short summary names each failure, and
each failure block opens with the `E ` lines of its message.
"""

from __future__ import annotations

import re
from collections.abc import Iterable

SECTION = re.compile(r"^=+ (.+?) =+$")
FAILURE_HEADER = re.compile(r"^_{3,} (.+?) _{3,}$")
RESULT = re.compile(r"^(FAILED|ERROR) ")
SUMMARY_SECTION = "short test summary info"


def excerpt(lines: Iterable[str], *, per_failure: int) -> str:
    """The short test summary, then the first `per_failure` `E ` lines of each
    failure or error block; empty when the lines are not pytest output."""
    if per_failure <= 0:
        return ""
    lines = list(lines)
    summary_at = next(
        (i for i, line in enumerate(lines) if SECTION.match(line) and SUMMARY_SECTION in line),
        None,
    )
    if summary_at is None:
        return ""
    summary = []
    for line in lines[summary_at + 1 :]:
        if SECTION.match(line):
            break
        if RESULT.match(line):
            summary.append(line)
    out = ["short test summary:", *(f"  {line}" for line in summary)]
    for name, kept in _failure_blocks(lines[:summary_at], per_failure):
        out.append(f"{name}:")
        out.extend(f"  {line}" for line in kept)
    return "\n".join(out)


def _failure_blocks(lines: list[str], per_failure: int) -> list[tuple[str, list[str]]]:
    blocks: list[tuple[str, list[str]]] = []
    name: str | None = None
    kept: list[str] = []

    def close() -> None:
        if name is not None and kept:
            blocks.append((name, kept))

    for line in lines:
        header = FAILURE_HEADER.match(line)
        if header:
            close()
            name, kept = header.group(1), []
        elif SECTION.match(line):
            close()
            name, kept = None, []
        elif name is not None and line.startswith("E ") and len(kept) < per_failure:
            kept.append(line)
    close()
    return blocks
