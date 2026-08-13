"""Load a checked-in script as a module without changing global import paths."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from typing import Any


def load_script(name: str, path: Path) -> Any:
    """Execute ``path`` as ``name`` and return its module object."""
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module
