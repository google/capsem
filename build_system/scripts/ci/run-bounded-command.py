#!/usr/bin/env python3
"""Compatibility launcher for the gate-owned bounded-command runner."""

import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
os.environ.setdefault("CAPSEM_REPOSITORY_ROOT", str(ROOT))
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / "build_system" / "builder"))
from bootstrap import mount_builder_package, reexec_project_python  # noqa: E402

reexec_project_python(ROOT, Path(__file__), sys.argv[1:])
mount_builder_package(ROOT)
from capsem_builder.gate.tools.ci import run_bounded_command as _implementation  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(_implementation.main())
