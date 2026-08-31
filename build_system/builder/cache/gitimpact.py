"""Read-only Git adapter for complete-test impact calculation."""

from __future__ import annotations

import subprocess
from pathlib import Path

from .models import GitImpact


def _git(root: Path, *arguments: str, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", *arguments],
        cwd=root,
        check=check,
        capture_output=True,
    )


def _paths(payload: bytes) -> tuple[str, ...]:
    if payload and not payload.endswith(b"\0"):
        raise RuntimeError("Git changed-path output was not NUL terminated")
    decoded = tuple(part.decode("utf-8") for part in payload.rstrip(b"\0").split(b"\0") if part)
    if any(path.startswith("/") or ".." in path.split("/") for path in decoded):
        raise RuntimeError("Git returned an invalid repository-relative path")
    return decoded


def inspect_git(root: Path, baseline: str, target: str | None) -> GitImpact:
    """Return ancestry, commit distance, and all tracked/untracked changed paths."""
    resolved = _git(root, "rev-parse", target or "HEAD").stdout.decode().strip()
    ancestor = _git(root, "merge-base", "--is-ancestor", baseline, resolved, check=False)
    if ancestor.returncode != 0:
        return GitImpact(
            baseline=baseline,
            target=resolved,
            ancestor=False,
            commits=0,
            paths=(),
        )
    commits = int(_git(root, "rev-list", "--count", f"{baseline}..{resolved}").stdout)
    comparison = f"{baseline}..{resolved}" if target is not None else baseline
    paths = set(_paths(_git(root, "diff", "--name-only", "-z", comparison).stdout))
    if target is None:
        paths.update(_paths(_git(root, "ls-files", "--others", "--exclude-standard", "-z").stdout))
    return GitImpact(
        baseline=baseline,
        target=resolved,
        ancestor=True,
        commits=commits,
        paths=tuple(sorted(paths)),
    )
