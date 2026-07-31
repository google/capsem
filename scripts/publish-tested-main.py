#!/usr/bin/env python3
"""Publish the exact clean main commit that completed the local release gate."""

from __future__ import annotations

import argparse
import subprocess
from collections.abc import Sequence
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _capture(*argv: str) -> str:
    return subprocess.run(
        argv,
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip()


def check_release_preconditions() -> None:
    """The publication requirements that do not depend on the gate having run.

    Checked twice on purpose: once before `just test` so an operator learns in
    seconds that their tree is dirty or their branch is wrong, and again at
    publication because the state can drift during a forty-minute gate. The
    rule lives here rather than being restated in the justfile, so the early
    check and the authoritative one cannot disagree.
    """
    if _capture("git", "status", "--porcelain", "--untracked-files=all"):
        raise RuntimeError("release requires the tested source tree to be clean")
    if _capture("git", "branch", "--show-current") != "main":
        raise RuntimeError("release must run from main")


def publish_tested_main(expected_head: str) -> None:
    if _capture("git", "rev-parse", "HEAD") != expected_head:
        raise RuntimeError("HEAD changed after the complete local release gate")
    check_release_preconditions()

    subprocess.run(("git", "fetch", "origin", "main"), cwd=ROOT, check=True)
    remote_head = _capture("git", "rev-parse", "origin/main")
    if remote_head == expected_head:
        return
    ancestor = subprocess.run(
        ("git", "merge-base", "--is-ancestor", "origin/main", expected_head),
        cwd=ROOT,
        check=False,
    )
    if ancestor.returncode != 0:
        raise RuntimeError("tested main diverged from origin/main; refusing to push")
    subprocess.run(("git", "push", "origin", "main"), cwd=ROOT, check=True)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected-head")
    parser.add_argument(
        "--precheck",
        action="store_true",
        help="Verify the publication preconditions and exit, without publishing. "
        "Run before the gate so a dirty tree fails in seconds, not after forty minutes.",
    )
    args = parser.parse_args(argv)
    if args.precheck:
        check_release_preconditions()
        return 0
    if not args.expected_head:
        parser.error("--expected-head is required unless --precheck is given")
    publish_tested_main(args.expected_head)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
