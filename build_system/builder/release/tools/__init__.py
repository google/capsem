"""Release command implementations owned by the build-system package."""

from __future__ import annotations

import os
from pathlib import Path


def repository_root() -> Path:
    """Return the checkout whose release surfaces invoked the package."""
    return Path(os.environ.get("CAPSEM_REPOSITORY_ROOT", Path.cwd())).resolve()
