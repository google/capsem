"""Compatibility adapter for release-owned channel authoring."""

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
from capsem_builder.release.tools.release_channel_author import *  # noqa: E402,F403
