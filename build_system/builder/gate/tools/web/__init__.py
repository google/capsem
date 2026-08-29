"""Gate-owned web and deployment command implementations."""

from __future__ import annotations

import os
from pathlib import Path


def repository_root() -> Path:
    """Return the checkout selected by a launcher or the current process."""
    return Path(os.environ.get("CAPSEM_REPOSITORY_ROOT", Path.cwd())).resolve()
