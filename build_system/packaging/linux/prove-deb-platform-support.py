#!/usr/bin/env python3
"""Launch the portable release-owned Debian platform proof."""

import importlib.util
import sys
from pathlib import Path


def main() -> int:
    if importlib.util.find_spec("capsem_builder") is None:
        root = Path(__file__).resolve().parents[3]
        sys.path.insert(0, str(root / "build_system/builder"))
        from bootstrap import mount_builder_package
        mount_builder_package(root)
    from capsem_builder.release.tools.prove_deb_platform_support import main as prove
    return prove()


if __name__ == "__main__":
    raise SystemExit(main())
