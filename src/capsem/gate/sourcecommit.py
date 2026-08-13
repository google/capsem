"""The immutable Git commit selected as a release's source."""

from __future__ import annotations

import re
import subprocess
from pathlib import Path
from typing import Self

from .errors import GateError

_FULL_COMMIT = re.compile(r"[0-9a-f]{40}\Z")


class SourceCommit(str):
    """One canonical full lowercase Git object id."""

    def __new__(cls, value: str) -> Self:
        if _FULL_COMMIT.fullmatch(value) is None:
            raise ValueError("source commit must be 40-character lowercase hexadecimal")
        return str.__new__(cls, value)


def _git(root: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args],
        cwd=root,
        check=check,
        capture_output=True,
        text=True,
    )


def require_local_main(root: Path, commit: SourceCommit) -> None:
    """Fail unless ``commit`` is an existing commit already on local main."""
    exists = _git(root, "cat-file", "-e", f"{commit}^{{commit}}", check=False)
    if exists.returncode != 0:
        raise GateError(f"source commit {commit} is not an existing local Git commit")
    contained = _git(root, "merge-base", "--is-ancestor", str(commit), "main", check=False)
    if contained.returncode != 0:
        raise GateError(f"source commit {commit} is not already on local main")


def require_detached_checkout(root: Path, commit: SourceCommit) -> None:
    """Prove a materialized source is detached at exactly ``commit``."""
    head = _git(root, "rev-parse", "HEAD", check=False)
    if head.returncode != 0 or head.stdout.strip() != commit:
        actual = head.stdout.strip() or "<unreadable>"
        raise GateError(f"exact source prefix HEAD {actual} does not match {commit}")
    branch = _git(root, "symbolic-ref", "-q", "HEAD", check=False)
    if branch.returncode == 0:
        raise GateError(f"exact source prefix is attached to {branch.stdout.strip()}")


def source_commit_for_checkout(root: Path) -> SourceCommit:
    """Return the canonical commit checked out at ``root``.

    Temporary local qualification graphs need the same package-row shape as
    public graphs, but they must not invent a sentinel SHA.  This is the one
    adapter from Git's text output into the typed release identity.
    """
    head = _git(root, "rev-parse", "HEAD", check=False)
    if head.returncode != 0:
        raise GateError(f"cannot resolve the source commit for {root}")
    try:
        return SourceCommit(head.stdout.strip())
    except ValueError as error:
        raise GateError(f"Git returned a noncanonical source commit for {root}") from error
