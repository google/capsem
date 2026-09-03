"""Load the one repository cache policy."""

from __future__ import annotations

import os
import tomllib
from collections.abc import Mapping
from pathlib import Path

from .models import CachePolicy
from .paths import CachePaths

POLICY_PATH = Path("config/cache.toml")


def _git_common_checkout(repository_root: Path) -> Path:
    """Return the primary checkout that owns a linked worktree's common Git dir."""
    marker = repository_root / ".git"
    if marker.is_dir() or not marker.is_file():
        return repository_root
    try:
        label, raw_git_dir = marker.read_text(encoding="utf-8").strip().split(":", 1)
        if label != "gitdir":
            return repository_root
        git_dir = Path(raw_git_dir.strip()).expanduser()
        if not git_dir.is_absolute():
            git_dir = repository_root / git_dir
        common_file = git_dir / "commondir"
        if not common_file.is_file():
            return repository_root
        common_git_dir = (git_dir / common_file.read_text(encoding="utf-8").strip()).resolve()
    except (OSError, ValueError):
        return repository_root
    return common_git_dir.parent if common_git_dir.name == ".git" else repository_root


def load_policy(repository_root: Path) -> CachePolicy:
    """Parse and validate the checked-in cache policy."""
    if not repository_root.is_absolute():
        raise ValueError("repository root must be absolute")
    with (repository_root / POLICY_PATH).open("rb") as stream:
        return CachePolicy.model_validate(tomllib.load(stream))


def load_paths(
    repository_root: Path,
    *,
    environment: Mapping[str, str] | None = None,
) -> CachePaths:
    """Resolve one policy and its optional shared-cache authority."""
    policy = load_policy(repository_root)
    values = os.environ if environment is None else environment
    configured = values.get(policy.authority_environment)
    authority = (
        _git_common_checkout(repository_root)
        if configured is None
        else Path(configured).expanduser()
    )
    if not authority.is_absolute():
        raise ValueError(f"cache authority from {policy.authority_environment} must be absolute")
    return CachePaths(repository_root=authority, policy=policy)
