#!/usr/bin/env python3
"""Compatibility launcher for the image-owned gate digest printer."""

import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
os.environ.setdefault("CAPSEM_REPOSITORY_ROOT", str(ROOT))
sys.path.insert(0, str(ROOT / "build_system" / "builder"))
from bootstrap import mount_builder_package, reexec_project_python  # noqa: E402

# The SessionStart hook runs this as `python3`, which is Apple's 3.9 on macOS;
# the digest printer needs the project interpreter (it imports tomllib).
reexec_project_python(ROOT, Path(__file__), sys.argv[1:])
mount_builder_package(ROOT)
from capsem_builder.image.tools.build import print_gate_digest as _implementation  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(_implementation.main())
