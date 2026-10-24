#!/usr/bin/env python3
"""Render the existing config-owned VM-signing roles for native packages."""

import sys
from pathlib import Path


def main() -> None:
    root = Path(__file__).resolve().parents[3]
    sys.path.insert(0, str(root / "build_system" / "builder"))
    from bootstrap import mount_builder_package, reexec_project_python

    reexec_project_python(root, Path(__file__).resolve(), sys.argv[1:])
    mount_builder_package(root)
    from capsem_builder.gate.config import load

    print("\n".join(Path(binary).name for binary in load(root).signing.binaries))


if __name__ == "__main__":
    main()
