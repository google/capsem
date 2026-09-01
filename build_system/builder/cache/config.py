"""Load the one repository cache policy."""

from __future__ import annotations

import tomllib
from pathlib import Path

from .models import CachePolicy

POLICY_PATH = Path("config/cache.toml")


def load_policy(repository_root: Path) -> CachePolicy:
    """Parse and validate the checked-in cache policy."""
    if not repository_root.is_absolute():
        raise ValueError("repository root must be absolute")
    with (repository_root / POLICY_PATH).open("rb") as stream:
        return CachePolicy.model_validate(tomllib.load(stream))
