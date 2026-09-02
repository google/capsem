"""Load the one repository cache policy."""

from __future__ import annotations

import os
import tomllib
from collections.abc import Mapping
from pathlib import Path

from .models import CachePolicy
from .paths import CachePaths

POLICY_PATH = Path("config/cache.toml")


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
    authority = repository_root if configured is None else Path(configured).expanduser()
    if not authority.is_absolute():
        raise ValueError(f"cache authority from {policy.authority_environment} must be absolute")
    return CachePaths(repository_root=authority, policy=policy)
