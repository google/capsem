#!/usr/bin/env python3
"""Use the existing package owner to embed/check candidate admission bytes."""

import importlib.util
import sys
from pathlib import Path


def main() -> int:
    root = Path(__file__).resolve().parents[3]
    if importlib.util.find_spec("capsem_builder") is None:
        sys.path.insert(0, str(root / "build_system/builder"))
        from bootstrap import mount_builder_package
        mount_builder_package(root)
    from capsem_builder.release.tools.preinstall_resource import main as run
    return run(repository=root)


if __name__ == "__main__":
    raise SystemExit(main())
