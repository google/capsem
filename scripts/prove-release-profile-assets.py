#!/usr/bin/env python3
"""Compatibility launcher for release-owned profile asset proof."""

import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault("CAPSEM_REPOSITORY_ROOT", str(ROOT))
try:
    import capsem_builder  # noqa: F401
except ModuleNotFoundError:
    sys.path.insert(0, str(ROOT / "build_system" / "builder"))
    from bootstrap import mount_builder_package
    mount_builder_package(ROOT)
from capsem_builder.release.tools.prove_release_profile_assets import main as _main  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(_main())
