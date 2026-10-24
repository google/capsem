#!/usr/bin/env python3
"""Resolve the native package destination through the admitted cache owner."""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
try:
    import capsem_builder  # noqa: F401
except ModuleNotFoundError:
    sys.path.insert(0, str(ROOT / "build_system" / "builder"))
    from bootstrap import mount_builder_package
    mount_builder_package(ROOT)
from capsem_builder.cache.config import load_paths  # noqa: E402

if __name__ == "__main__":
    print(load_paths(ROOT).stage("packages"))
