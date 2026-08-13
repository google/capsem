#!/usr/bin/env python3
"""Classify the one CI shortcut from Git's exact changed-path stream.

The shortcut saves only product builds.  The fast gate is unconditional, and
anything executable, ambiguous, or newly introduced outside the narrow web
content roots fails closed into the full CI matrix.
"""

from __future__ import annotations

import os
import sys
from collections.abc import Iterable

CONTENT_FILES = frozenset({"README.md"})
CONTENT_PREFIXES = ("docs/", "later/", "site/")
PUBLIC_INSTALLERS = frozenset({"docs/public/install.sh", "site/public/install.sh"})


def paths_from_git(payload: bytes) -> tuple[str, ...]:
    """Decode `git diff --name-only -z`, rejecting every other framing."""
    if not payload:
        return ()
    if not payload.endswith(b"\0"):
        raise ValueError("changed paths must be NUL-terminated Git output")
    encoded = payload[:-1].split(b"\0")
    if any(not path for path in encoded):
        raise ValueError("changed paths contain an empty NUL-delimited entry")
    return tuple(os.fsdecode(path) for path in encoded)


def web_only(paths: Iterable[str]) -> bool:
    """Whether every changed path is inert web content.

    Non-emptiness is part of the proof.  An empty diff can mean a bad base or
    a broken classifier; neither is permission to skip product jobs.
    """
    changed = tuple(paths)
    if not changed:
        return False
    for path in changed:
        if path in PUBLIC_INSTALLERS:
            return False
        if path in CONTENT_FILES:
            continue
        if path.startswith(CONTENT_PREFIXES):
            continue
        return False
    return True


def main() -> int:
    try:
        changed = paths_from_git(sys.stdin.buffer.read())
    except ValueError as failure:
        print(f"CI scope classification failed: {failure}", file=sys.stderr)
        return 2
    print("true" if web_only(changed) else "false")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
