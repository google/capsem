#!/usr/bin/env python3
"""Verify and publish the immutable source ref used by release workflows."""

from __future__ import annotations

import argparse
import re
import subprocess
from collections.abc import Sequence
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
COMMIT = re.compile(r"[0-9a-f]{40}\Z")


def _run(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=ROOT, check=check, capture_output=True, text=True)


def _capture(*args: str) -> str:
    return _run(*args).stdout.strip()


def _require_source(commit: str) -> None:
    if COMMIT.fullmatch(commit) is None:
        raise RuntimeError("source commit must be 40-character lowercase hexadecimal")
    if _capture("rev-parse", "HEAD") != commit:
        raise RuntimeError(f"detached release source is not {commit}")
    if _run("symbolic-ref", "-q", "HEAD", check=False).returncode == 0:
        raise RuntimeError("release source must be detached from every mutable branch")
    if _capture("status", "--porcelain", "--untracked-files=all"):
        raise RuntimeError("release source prefix is not clean")


def _require_remote_main(commit: str) -> None:
    _run("fetch", "--no-tags", "origin", "+refs/heads/main:refs/remotes/origin/main")
    if _run(
        "merge-base", "--is-ancestor", commit, "refs/remotes/origin/main", check=False
    ).returncode:
        raise RuntimeError(f"source commit {commit} is not already on fresh origin/main")


def _remote_ref(full_ref: str) -> str | None:
    rows = [
        line.split() for line in _capture("ls-remote", "--refs", "origin", full_ref).splitlines()
    ]
    if not rows:
        return None
    if len(rows) != 1 or len(rows[0]) != 2 or rows[0][1] != full_ref:
        raise RuntimeError(f"remote returned malformed or duplicate source ref rows: {rows}")
    return rows[0][0]


def publish(commit: str, template: str) -> str:
    _require_source(commit)
    _require_remote_main(commit)
    short_ref = template.format(source_commit=commit)
    if not re.fullmatch(r"capsem-source-[0-9a-f]{40}", short_ref):
        raise RuntimeError("release source ref template produced an unsafe ref")
    full_ref = f"refs/tags/{short_ref}"
    existing = _remote_ref(full_ref)
    if existing is not None and existing != commit:
        raise RuntimeError(f"immutable source ref {full_ref} points at {existing}, not {commit}")
    if existing is None:
        pushed = _run("push", "origin", f"{commit}:{full_ref}", check=False)
        # A same-value concurrent creator is harmless; the authoritative
        # re-read below decides. A different value remains an error.
        if pushed.returncode != 0 and _remote_ref(full_ref) != commit:
            raise RuntimeError(pushed.stderr.strip() or f"could not create {full_ref}")
    confirmed = _remote_ref(full_ref)
    if confirmed != commit:
        raise RuntimeError(f"source ref {full_ref} did not resolve to {commit}")
    return short_ref


def check(commit: str) -> None:
    _require_source(commit)
    _require_remote_main(commit)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source_commit")
    parser.add_argument("--ref-template", required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)
    try:
        if args.check:
            check(args.source_commit)
        else:
            print(publish(args.source_commit, args.ref_template))
    except (OSError, subprocess.CalledProcessError, RuntimeError) as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
