#!/usr/bin/env python3
"""Publish the exact clean main commit that completed the local release gate."""

from __future__ import annotations

import argparse
from pathlib import Path
import subprocess
from typing import Sequence


ROOT = Path(__file__).resolve().parents[1]


def _capture(*argv: str) -> str:
    return subprocess.run(
        argv,
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip()


def publish_tested_main(expected_head: str) -> None:
    if _capture("git", "rev-parse", "HEAD") != expected_head:
        raise RuntimeError("HEAD changed after the complete local release gate")
    if _capture("git", "status", "--porcelain", "--untracked-files=all"):
        raise RuntimeError("release requires the tested source tree to be clean")
    if _capture("git", "branch", "--show-current") != "main":
        raise RuntimeError("release must run from main")

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
    parser.add_argument("--expected-head", required=True)
    args = parser.parse_args(argv)
    publish_tested_main(args.expected_head)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
