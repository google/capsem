#!/usr/bin/env python3
"""Render the existing config-owned VM-signing roles for native packages."""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "build_system" / "builder"))
from bootstrap import mount_builder_package, reexec_project_python  # noqa: E402

reexec_project_python(ROOT, Path(__file__).resolve(), sys.argv[1:])
mount_builder_package(ROOT)
from capsem_builder.gate.config import load  # noqa: E402

if __name__ == "__main__":
    print("\n".join(Path(binary).name for binary in load(ROOT).signing.binaries))
