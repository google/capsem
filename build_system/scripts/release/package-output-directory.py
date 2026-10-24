#!/usr/bin/env python3
"""Resolve the native package destination through the admitted cache owner."""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "build_system" / "builder"))
from bootstrap import mount_builder_package, reexec_project_python  # noqa: E402

reexec_project_python(ROOT, Path(__file__).resolve(), sys.argv[1:])
mount_builder_package(ROOT)
from capsem_builder.cache.config import load_paths  # noqa: E402

if __name__ == "__main__":
    print(load_paths(ROOT).stage("packages"))
