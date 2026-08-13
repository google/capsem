"""Materialize an immutable release commit without reading working-tree bytes."""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

from .config import GateConfig
from .errors import GateError
from .filesystem import copy_tree, remove
from .sourcecommit import SourceCommit, require_local_main


def _git(root: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=root, check=check, capture_output=True, text=True)


def _origin_url(source: Path) -> str:
    """Return an origin whose meaning survives moving into the prefix."""
    result = _git(source, "remote", "get-url", "origin", check=False)
    value = result.stdout.strip()
    if result.returncode != 0 or not value:
        raise GateError("release source checkout has no canonical origin remote")
    # A filesystem remote may be relative to the outer worktree. Copying that
    # spelling into a differently located clone can silently retarget release
    # fetches and pushes. URL and scp-like remotes are already location-stable.
    if "://" not in value and ":" not in value and not Path(value).is_absolute():
        return str((source / value).resolve())
    return value


def _copy_inputs(source: Path, target: Path, config: GateConfig) -> None:
    """Copy the one declared ignored release input, never repository metadata."""
    signing = config.package.signing.directory
    ignored = _git(target, "check-ignore", "-q", signing, check=False)
    if ignored.returncode != 0:
        raise GateError(f"release signing input {signing} is not ignored by {target}")
    for relative in (signing,):
        origin = source / relative
        destination = target / relative
        if destination.exists() or destination.is_symlink():
            remove(destination)
        if not origin.exists():
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        if origin.is_dir():
            copy_tree(origin, destination, symlinks=True)
        else:
            shutil.copy2(origin, destination, follow_symlinks=False)


def _checkout(source: Path, target: Path, config: GateConfig, commit: SourceCommit) -> None:
    require_local_main(source, commit)
    _git(target, "checkout", "--detach", "--force", str(commit))
    _git(target, "clean", "-fdx")
    _copy_inputs(source, target, config)
    head = _git(target, "rev-parse", "HEAD").stdout.strip()
    if head != commit:
        raise GateError(f"exact source prefix resolved {head}, expected {commit}")
    dirty = _git(target, "status", "--porcelain", "--untracked-files=all").stdout.strip()
    if dirty:
        raise GateError(f"exact source prefix {target} is not clean after checkout: {dirty}")


def populate(source: Path, target: Path, config: GateConfig, commit: SourceCommit) -> None:
    """Create a private detached checkout of ``commit`` plus carried secrets."""
    if target.exists() and any(target.iterdir()):
        raise GateError(f"exact source prefix {target} is not empty")
    target.parent.mkdir(parents=True, exist_ok=True)
    origin = _origin_url(source)
    subprocess.run(
        ["git", "clone", "--no-local", "--no-checkout", str(source), str(target)],
        check=True,
        capture_output=True,
        text=True,
    )
    _git(target, "remote", "set-url", "origin", origin)
    _checkout(source, target, config, commit)
