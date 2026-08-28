#!/usr/bin/env python3
"""Run the strict typed runtime-preflight selector from a source checkout."""

from __future__ import annotations

import runpy
import sys
from pathlib import Path

if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    sys.path.insert(0, str(root / "build_system" / "builder"))
    from bootstrap import mount_builder_package

    mount_builder_package(root)
    runpy.run_module(
        "capsem_builder.release.runtime_preflight_manifest", run_name="__main__"
    )
